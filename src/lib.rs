use std::{ffi::OsStr, os::raw::c_int, str::FromStr, sync::atomic::Ordering};

use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::{
    client::docs::{Doc, Entry},
    docs::{store::Query, NamespaceId},
    net::ticket::NodeTicket,
    node::Node,
};
use tokio::fs::{self, OpenOptions};

pub mod prelude;
use prelude::*;

mod manifest;
pub use manifest::Manifest;

mod util;
use util::*;

mod cli;
pub use cli::{Cli, Commands};

mod fuse;
use fuse::{check_access, clear_suid_sgid, FileKind, InodeAttributes};

mod object;
use object::Object;

// mod directory;
// use directory::Directory;

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    pub manifest: Manifest,
    pub rt: tokio::runtime::Handle,
    root_doc: Doc,
    pub root: PathBuf,
}

impl Lis {
    /// Creates new Lis node
    /// If `root` path does not exist, it is created with `mkdir -p`
    /// If an Iroh node is found in `root`, a new one is created
    /// If an Iroh node is found in `root` but `overwrite` is `true`, the old one is truncated
    pub async fn new(root: &PathBuf, overwrite: bool) -> Result<Self> {
        if overwrite {
            // TODO: add prompt for overwrite: are you sure? [Y/n]

            // remove old root dir if one existed before
            let _ = fs::remove_dir_all(root); // don't care about result. if dir not exists it's
                                              // fine
        }
        // create root if not exists
        fs::create_dir_all(root.clone()).await?;
        // TODO: also create dir for lis (iroh) metadata inside root if not exists
        // let metadata_dir = root.join(".lis");
        // fs::create_dir_all(metadata_dir.clone())?;

        let iroh_node = iroh::node::Node::persistent(root).await?.spawn().await?;
        // if manifest.json file found, load it
        // manifest.json holds data about the Files document (which points to all files)
        let manifest_path = root.join("manifest.json");
        // TODO: let manifest_path = metadata_dir.join("manifest.json");

        let (manifest, root_doc) = match Manifest::load(&manifest_path)? {
            Some(manifest) => {
                let root_doc = iroh_node
                    .docs()
                    .open(NamespaceId::from_str(manifest.root_doc_id.as_str())?)
                    .await?
                    .ok_or_else(|| anyhow!("no files doc found"))?;
                (manifest, root_doc)
            }
            None => {
                // create new Files doc and manifest file
                let root_doc = iroh_node.docs().create().await?;
                let author = iroh_node.authors().create().await?;
                iroh_node.authors().set_default(author).await?;

                let manifest = Manifest::new(manifest_path, root_doc.id().to_string())?;

                manifest.save()?;

                (manifest, root_doc)
            }
        };

        let lis = Lis {
            iroh_node,
            manifest,
            rt: tokio::runtime::Handle::current(),
            root_doc,
            root: root.clone(),
        };
        Ok(lis)
    }

    /// Creates a new inode for use
    pub fn next_ino(&mut self) -> Inode {
        let ino = self.manifest.cur_ino.fetch_add(1, Ordering::SeqCst).into();
        self.manifest
            .save()
            .expect("could not write to manifest file");
        ino
    }

    /// Creates a new file handle for use
    pub fn next_file_handle(&mut self, read: bool, write: bool) -> FileHandle {
        let mut fh = self.manifest.cur_fh.fetch_add(1, Ordering::SeqCst).into();
        // Assert that we haven't run out of file handles
        assert!(fh < FILE_HANDLE_READ_BIT.min(FILE_HANDLE_WRITE_BIT));
        if read {
            fh |= FILE_HANDLE_READ_BIT;
        }
        if write {
            fh |= FILE_HANDLE_WRITE_BIT;
        }
        self.manifest
            .save()
            .expect("could not write to manifest file");

        fh
    }

    /// Create new empty file on lis
    pub async fn touch(
        &mut self,
        full_path: &PathBuf,
        mode: Option<u16>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<()> {
        // find doc where file will live
        let doc = self
            .find_dir_doc(
                &full_path
                    .parent()
                    .ok_or(anyhow!("Could not find Doc for parent dir"))?
                    .to_path_buf(),
            )
            .await?;

        let key = key_from_file(Path::new(""), &full_path)?;

        // if key already in filesystem, do nothing and return Ok
        let query = Query::key_exact(key.clone());
        if doc.get_one(query).await?.is_some() {
            return Ok(());
        }

        let default_author = self.iroh_node.authors().default().await?;
        let content = b"null"; //cannot be b"" because iroh will think it's a deleted file
        doc.set_bytes(default_author, key.to_vec(), content.to_vec())
            .await?;

        // add file obj to filesystem
        let size: u64 = 4;
        self.create_fs_objects(&full_path, FileKind::File, Some(size), mode, uid, gid)?;

        Ok(())
    }

    /// List all files in node
    pub async fn list(&self, full_path: &Path) -> Result<Vec<Result<Entry>>> {
        let mut path = full_path;

        if path.starts_with("/") {
            path = path.strip_prefix("/")?;
        }

        // iterate until last dir
        let mut doc = self.root_doc.clone();
        for dir in path.iter() {
            doc = match self.next_doc(&doc, Path::new(dir)).await? {
                Some(next_doc) => next_doc,
                None => {
                    return Err(anyhow!(
                        "could not find {} in tree",
                        Path::new(dir).display()
                    ))
                }
            };
        }

        let query = Query::all().build();
        let entries = doc.get_many(query).await?.collect::<Vec<_>>().await;

        Ok(entries)
    }

    pub fn obj_from_path(&self, full_path: &Path) -> Option<&Object> {
        let ino = self.manifest.inodes.get(full_path)?;
        self.manifest.objects.get(&ino)
    }

    pub fn write_inode(&mut self, attrs: &InodeAttributes) -> Result<()> {
        let ino: Inode = attrs.inode;
        match self.manifest.objects.get_mut(&ino) {
            Some(obj) => {
                obj.attrs = attrs.clone();
                self.manifest.save()
            }
            None => Err(anyhow!("could not find object for inode {ino}")),
        }
    }

    /// Adds files and directories to Lis
    /// Returns `(path, key)` pairs of the added file upon success
    pub async fn put(
        &mut self,
        src_path: &Path,
        dst_path: &Path,
    ) -> Result<Vec<(PathBuf, String)>> {
        if !src_path.exists() {
            return Err(anyhow!("Path {} does not exist", src_path.display()));
        }

        let full_src_path = fs::canonicalize(&src_path).await?;
        if !full_src_path.is_file() {
            return Err(anyhow!("{} is not a file", full_src_path.display()));
        }
        let full_dst_path = add_leading_slash(dst_path);

        let doc = self
            .find_dir_doc(
                &dst_path
                    .parent()
                    .ok_or(anyhow!("Could not find Doc for parent dir"))?
                    .to_path_buf(),
            )
            .await?;

        self.put_file_to_doc(&doc, full_src_path.as_path(), &full_dst_path)
            .await?;

        // add file obj to filesystem
        let size = fs::metadata(src_path).await?.len();
        self.create_fs_objects(&full_dst_path, FileKind::File, Some(size), None, None, None)?;

        Ok(vec![(
            src_path.to_path_buf(),
            full_dst_path.to_string_lossy().to_string(),
        )])
    }

    /// Puts a file to a previously created document
    /// Returns the key to the added file upon success
    async fn put_file_to_doc(
        &mut self,
        doc: &Doc,
        import_path: &Path,
        dst_path: &Path,
    ) -> Result<()> {
        let key = key_from_file(Path::new(""), dst_path)?;

        let default_author = self.iroh_node.authors().default().await?;

        // if key already in filesystem, remove it first
        let query = Query::key_exact(key.clone());
        if doc.get_one(query).await?.is_some() {
            doc.del(default_author, key.clone()).await?; // delete old entry
        }

        doc.import_file(default_author, key.clone(), import_path, false)
            .await?
            .collect::<Vec<_>>()
            .await;

        Ok(())
    }

    fn create_fs_objects(
        &mut self,
        full_path: &Path,
        kind: FileKind,
        size: Option<u64>,
        mode: Option<u16>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<()> {
        let inode = self.next_ino();
        let obj = Object::new(full_path, inode, kind, size, mode, uid, gid)?;

        self.manifest.objects.insert(inode, obj);
        self.manifest.inodes.insert(full_path.to_path_buf(), inode);

        debug!("Created {} (ino={inode})", full_path.display());

        self.manifest.save()?;

        Ok(())
    }

    /// Remove a file
    pub async fn rm_file(&mut self, path: &Path) -> Result<String> {
        let key = key_from_file(Path::new(""), path)?;

        self.root_doc
            .del(self.iroh_node.authors().default().await?, key.clone())
            .await?;
        key_to_string(key)
    }

    /// Get contents of a file
    pub async fn get_file(&mut self, full_path: &Path) -> Result<Bytes> {
        let key = key_from_file(Path::new(""), full_path)?;

        // get content of the key from doc
        let query = Query::key_exact(key);
        let doc = self
            .find_dir_doc(
                &full_path
                    .parent()
                    .ok_or(anyhow!("Could not find Doc for parent dir"))?
                    .to_path_buf(),
            )
            .await?;
        let entry = doc
            .get_one(query)
            .await?
            .ok_or_else(|| anyhow!("entry not found"))?;

        entry.content_bytes(self.iroh_node.client()).await
    }

    /// Generate a NodeTicket invite
    pub async fn invite(&self) -> Result<NodeTicket> {
        let node_addr = self.iroh_node.net().node_addr().await?;
        NodeTicket::new(node_addr)
    }
    /// Joins a network from a NodeTicket invite
    pub fn join(&mut self, ticket: &NodeTicket) -> Result<()> {
        let endpoint = self.iroh_node.endpoint();
        endpoint.add_node_addr(ticket.node_addr().clone())
    }

    fn get_full_path(&self, parent: Inode, name: &OsStr) -> Result<PathBuf> {
        let name = PathBuf::from(name);
        let parent_obj = self
            .manifest
            .objects
            .get(&parent)
            .ok_or(anyhow!("could not find parent's inode"))?;
        Ok(parent_obj.full_path.join(name))
    }

    /// Create directory if doesn't already exist
    pub async fn mkdir(
        &mut self,
        full_path: &PathBuf,
        mode: Option<u16>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<NamespaceId> {
        // find parent dir
        // if we're creating /1/2/3, this will find the doc of /1/2
        let parent_doc = self
            .find_dir_doc(
                &full_path
                    .parent()
                    .ok_or(anyhow!("Could not find Doc for parent dir"))?
                    .to_path_buf(),
            )
            .await?;

        let relpath = Path::new(
            full_path
                .file_name()
                .ok_or(anyhow!("Could not get last dir name"))?,
        );

        // create doc representing dir
        let doc = self.create_doc(&parent_doc, relpath).await?;

        // add needed objects to fs structure (fuse)
        self.create_fs_objects(full_path, FileKind::Directory, None, mode, uid, gid)?;
        debug!("Created directory {}", full_path.display());

        Ok(doc.id())
    }

    async fn find_dir_doc(&self, full_path: &PathBuf) -> Result<Doc> {
        // strip leading / from path
        let mut path = full_path.clone();
        if path.starts_with("/") {
            path = path.strip_prefix("/")?.to_path_buf();
        }

        // iterate until last dir
        let mut doc = self.root_doc.clone();
        for dir in &path {
            doc = match self.next_doc(&doc, Path::new(dir)).await? {
                Some(next_doc) => next_doc,
                None => {
                    return Err(anyhow!(
                        "could not find {} in tree",
                        Path::new(dir).display()
                    ))
                }
            };
        }
        Ok(doc)
    }

    /// Gets next doc from base_doc and key
    async fn next_doc(&self, base_doc: &Doc, next_key: &Path) -> Result<Option<Doc>> {
        let key = key_from_file(Path::new(""), next_key)?;
        let query = Query::key_exact(key.clone());
        let next_doc_id = match base_doc.get_one(query).await? {
            Some(entry) => entry.content_bytes(base_doc).await?,
            None => return Ok(None),
        };

        Ok(self
            .iroh_node
            .docs()
            .open(bytes_to_namespaceid(next_doc_id)?)
            .await?)
    }
    /// Creates new Doc with name `next_key` and `base_doc` as its parent
    async fn create_doc(&mut self, base_doc: &Doc, dir_name: &Path) -> Result<Doc> {
        // check if key already exists in base_doc
        let key = key_from_file(Path::new(""), dir_name)?;
        let query = Query::key_exact(key.clone());
        if let Some(_doc_id) = base_doc.get_one(query).await? {
            return Err(anyhow!("cannot create directory, already exists"));
        }

        // Doc doesn't already exist, create new Doc
        let new_doc = self.iroh_node.docs().create().await?;
        let author = self.iroh_node.authors().default().await?;
        base_doc
            .set_bytes(author, key, namespaceid_to_bytes(new_doc.id()))
            .await?;

        Ok(new_doc)
    }

    async fn truncate(
        &mut self,
        ino: Inode,
        new_length: u64,
        uid: u32,
        gid: u32,
    ) -> Result<InodeAttributes, c_int> {
        if new_length > MAX_FILE_SIZE {
            return Err(libc::EFBIG);
        }

        let (mut attrs, obj) = match self.manifest.objects.get(&ino) {
            Some(obj) => (obj.attrs.clone(), obj),
            None => {
                return Err(libc::ENOENT);
            }
        };

        if !check_access(attrs.uid, attrs.gid, attrs.mode, uid, gid, libc::W_OK) {
            return Err(libc::EACCES);
        }

        let system_full_path = fuse::system_full_path(&self.root, &obj.full_path);
        debug!("system_full_path={}", system_full_path.display());
        let file = match OpenOptions::new()
            .write(true)
            .open(system_full_path.clone())
            .await
        {
            Ok(file) => file,
            Err(_e) => return Err(libc::ENOENT),
        };
        debug!("opened {}", system_full_path.display());
        file.set_len(new_length).await.unwrap();

        attrs.size = new_length;
        attrs.last_metadata_changed = SystemTime::now();
        attrs.last_modified = SystemTime::now();

        // Clear SETUID & SETGID on truncate
        clear_suid_sgid(&mut attrs);

        if let Err(e) = self.write_inode(&attrs) {
            error!("Could not truncate: {e}");
            return Err(libc::ENOENT);
        }

        Ok(attrs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    async fn setup_lis(tmp_dir: &TempDir) -> Lis {
        let root = PathBuf::from(tmp_dir.path());
        let overwrite = true;
        Lis::new(&root, overwrite)
            .await
            .expect("Could not create new Lis node")
    }

    #[tokio::test]
    async fn put_dir() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        let file_path = tmp_dir.path();
        let mut file = NamedTempFile::new_in(file_path).expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");
        let src_path = file.path();
        let dst_path = Path::new(file.path().file_name().unwrap());

        lis.put(src_path, dst_path)
            .await
            .expect("Could not put file"); // should succeed
        let get_content = lis.get_file(dst_path).await.expect("Could not get file"); // should succeed
        assert_eq!(get_content, content);
    }

    #[tokio::test]
    async fn double_put() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");
        let binding = file.path().to_path_buf();
        let dst_path = Path::new(binding.file_name().unwrap());

        // put file twice
        lis.put(file.path(), dst_path)
            .await
            .expect("Could not put file"); // should succeed

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");

        lis.put(file.path(), dst_path)
            .await
            .expect("Could not put file");

        let get_content = lis.get_file(dst_path).await.expect("Could not get file"); // should succeed

        let files = lis.list(Path::new("/")).await.expect("Could not get file"); // should succeed

        assert_eq!(get_content, "Brian was here. Briefly. more"); // new content should be there
        assert_eq!(files.len(), 1); // there should only be one file
    }

    #[tokio::test]
    async fn put_file() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");

        let binding = file.path().to_path_buf();
        let dst_path = Path::new(binding.file_name().unwrap());

        // put file twice
        lis.put(file.path(), dst_path)
            .await
            .expect("Could not put file");

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");
        lis.put(file.path(), dst_path)
            .await
            .expect("Could not put file");

        let get_content = lis.get_file(dst_path).await.expect("Could not get file"); // should succeed

        let files = lis.list(Path::new("/")).await.expect("Could not get file"); // should succeed

        assert_eq!(get_content, "Brian was here. Briefly. more"); // new content should be there
        assert_eq!(files.len(), 1); // there should only be one file
    }

    #[tokio::test]
    async fn mkdir() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").unwrap();
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).unwrap();

        // create dir structure
        // check if 1 was created in /
        lis.mkdir(&Path::new("/1").to_path_buf(), None, None, None)
            .await
            .unwrap();
        let entries = lis.list(Path::new("/")).await.unwrap(); // should succeed
        assert_eq!(entries.len(), 1); // there should only be one dir

        // check if 2 was created in /1
        lis.mkdir(&Path::new("/1/2").to_path_buf(), None, None, None)
            .await
            .unwrap();
        let entries = lis.list(Path::new("/1")).await.unwrap(); // should succeed
        assert_eq!(entries.len(), 1); // there should only be one dir

        // check if 3 was created in /1/2
        lis.mkdir(&Path::new("/1/2/3").to_path_buf(), None, None, None)
            .await
            .unwrap();
        let entries = lis.list(Path::new("/1/2")).await.unwrap(); // should succeed
        assert_eq!(entries.len(), 1); // there should only be one dir

        // add file /1/2/3/myfile.txt
        let src_path = file.path();
        let dst_path = Path::new("/1/2/3").join(file.path().file_name().unwrap());

        // put file twice
        lis.put(src_path, &dst_path)
            .await
            .expect("Could not put file");

        // check if file was created in path
        let files = lis.list(Path::new("/1/2/3")).await.unwrap(); // should succeed
        assert_eq!(files.len(), 1); // there should be exactly one file

        // retrieve content from the file
        let get_content = lis.get_file(&dst_path).await.unwrap(); // should succeed
        assert_eq!(get_content, "Brian was here. Briefly."); // new content should be there
    }

    #[tokio::test]
    async fn touch() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create empty file (touch) in lis
        let file_path = Path::new("/myfile.txt");
        lis.touch(&file_path.to_path_buf(), None, None, None)
            .await
            .unwrap();

        // retrieve content from the file (should be b"null")
        let get_content = lis.get_file(file_path).await.unwrap();
        assert_eq!(get_content, "null");
    }
}

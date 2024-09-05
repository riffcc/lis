use std::{ffi::OsStr, fs, str::FromStr, sync::atomic::Ordering};

use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::{
    client::docs::{Doc, Entry},
    docs::{store::Query, NamespaceId},
    net::ticket::NodeTicket,
    node::Node,
};

pub mod prelude;
use prelude::*;

mod manifest;
pub use manifest::Manifest;

mod util;
use util::{get_paths_in_dir, key_from_file, key_to_string};

mod cli;
pub use cli::{Cli, Commands};

mod fuse;

mod object;
use object::Object;

mod directory;
use directory::DirTree;

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    pub manifest: Manifest,
    files_doc: Doc,
    _root: PathBuf,
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
        // create root path if not exists
        fs::create_dir_all(root)?;

        let iroh_node = iroh::node::Node::persistent(root).await?.spawn().await?;
        // if manifest.json file found, load it
        // manifest.json holds data about the Files document (which points to all files)
        let manifest_path = root.join("manifest.json");

        let (manifest, files_doc) = match Manifest::load(&manifest_path)? {
            Some(manifest) => {
                let files_doc = iroh_node
                    .docs()
                    .open(NamespaceId::from_str(manifest.files_doc_id.as_str())?)
                    .await?
                    .ok_or_else(|| anyhow!("no files doc found"))?;
                (manifest, files_doc)
            }
            None => {
                // create new Files doc and manifest file
                let files_doc = iroh_node.docs().create().await?;
                let author = iroh_node.authors().create().await?;
                iroh_node.authors().set_default(author).await?;

                let manifest = Manifest::new(manifest_path, files_doc.id().to_string())?;

                manifest.save()?;

                (manifest, files_doc)
            }
        };

        let lis = Lis {
            iroh_node,
            manifest,
            files_doc,
            _root: root.clone(),
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

    /// Returns the Doc struct if a path is a directory
    pub fn path_to_doc(&self, path: &Path) -> Option<Doc> {
        // TODO: implement
        None
    }

    /// List all files in node
    pub async fn list(&self) -> Result<Vec<Result<Entry>>> {
        let query = Query::all().build();
        let entries = self
            .files_doc
            .get_many(query)
            .await?
            .collect::<Vec<_>>()
            .await;

        Ok(entries)
    }

    pub fn obj_from_path(&self, path: &Path) -> Option<&Object> {
        if let Some(ino) = self.manifest.inodes.get(path) {
            self.manifest.objects.get(&ino)
        } else {
            None
        }
    }

    /// Adds files and directories to Lis
    /// Returns `(path, key)` pairs of the added file upon success
    pub async fn put(&mut self, src_path: &Path) -> Result<Vec<(PathBuf, String)>> {
        if !src_path.exists() {
            return Err(anyhow!("Path {} does not exist", src_path.display()));
        }

        let full_src_path = fs::canonicalize(&src_path)?;
        if src_path.is_file() {
            Ok(vec![(
                src_path.to_path_buf(),
                self.put_file_to_doc(full_src_path.as_path()).await?,
            )])
        } else {
            let paths = get_paths_in_dir(&full_src_path)?;

            let mut entries = Vec::new();
            for path in paths {
                entries.push(self.put_file(&path).await?);
            }
            self.manifest.save()?;

            Ok(entries)
        }
    }

    /// Creates a new Doc and adds a file to it
    /// Returns a `(path, key)` pair of the added file upon success
    async fn put_file(&mut self, src_path: &Path) -> Result<(PathBuf, String)> {
        if !src_path.exists() {
            return Err(anyhow!("File {} not found", src_path.display()));
        }
        if !src_path.is_file() {
            return Err(anyhow!("Path must be a file"));
        }

        let full_src_path = fs::canonicalize(&src_path)?;
        Ok((
            src_path.to_path_buf(),
            self.put_file_to_doc(full_src_path.as_path()).await?,
        ))
    }

    /// Puts a file to a previously created document
    /// Returns the key to the added file upon success
    async fn put_file_to_doc(&mut self, path: &Path) -> Result<String> {
        let key = key_from_file(Path::new(""), path)?;

        // if key already in filesystem, remove it first
        let query = Query::key_exact(key.clone());
        if self.files_doc.get_one(query).await?.is_some() {
            self.files_doc
                .del(self.iroh_node.authors().default().await?, key.clone())
                .await?; // delete old entry
        }

        self.files_doc
            .import_file(
                self.iroh_node.authors().default().await?,
                key.clone(),
                path,
                false,
            )
            .await?
            .collect::<Vec<_>>()
            .await;

        let str_key = key_to_string(key);
        if let Ok(ref skey) = str_key {
            let inode = self.next_ino();
            let obj = Object::new(path, inode);
            debug!("Adding {skey} (ino={inode})");
            self.manifest.objects.insert(inode, obj?);
            self.manifest
                .inodes
                .insert(PathBuf::from(skey.replace("\0", "")), inode);
            self.manifest.save()?;
        }
        str_key
    }

    /// Remove a file
    pub async fn rm_file(&mut self, path: &Path) -> Result<String> {
        let key = key_from_file(Path::new(""), path)?;

        self.files_doc
            .del(self.iroh_node.authors().default().await?, key.clone())
            .await?;
        key_to_string(key)
    }

    /// Get contents of a file
    pub async fn get_file(&mut self, path: &Path) -> Result<Bytes> {
        let key = key_from_file(Path::new(""), path)?;

        // get content of the key from doc
        let query = Query::key_exact(key);
        let entry = self
            .files_doc
            .get_one(query)
            .await?
            .ok_or_else(|| anyhow!("entry not found"))?;

        entry.content_bytes(self.iroh_node.client()).await
    }

    /// Creates directory (if `recursive` is `true` create the full path)
    pub async fn mkdir(path: &Path, recursive: bool) {
        for doc in self.doc_from_path(create_dir)
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
        Ok(parent_obj.path.join(name))
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

        lis.put(file.path()).await.expect("Could not put file"); // should succeed
        let get_content = lis.get_file(file.path()).await.expect("Could not get file"); // should succeed
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

        // put file twice
        lis.put(file.path()).await.expect("Could not put file"); // should succeed

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");
        lis.put(file.path()).await.expect("Could not put file"); // should succeed

        let get_content = lis.get_file(file.path()).await.expect("Could not get file"); // should succeed

        let files = lis.list().await.expect("Could not get file"); // should succeed

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

        // put file twice
        lis.put(file.path()).await.expect("Could not put file"); // should succeed

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");
        lis.put(file.path()).await.expect("Could not put file"); // should succeed

        let get_content = lis.get_file(file.path()).await.expect("Could not get file"); // should succeed

        let files = lis.list().await.expect("Could not get file"); // should succeed

        assert_eq!(get_content, "Brian was here. Briefly. more"); // new content should be there
        assert_eq!(files.len(), 1); // there should only be one file
    }
}

use iroh::{
    // client::docs::{Doc, Entry},
    // docs::{store::Query, NamespaceId},
    // net::ticket::NodeTicket,
    node::Node,
};
use tokio::fs;

pub mod prelude;
use prelude::*;

mod util;
//use util::*;

mod cli;
pub use cli::{Cli, Commands};

mod objects;
use objects::{dir::Dir as LisDir, file::File as LisFile, Root};

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    iroh_dir: PathBuf,
    pub rt: tokio::runtime::Handle,
    root: Root,
}

impl Lis {
    /// Creates new Lis node
    /// If `iroh_dir` path does not exist, it is created with `mkdir -p`
    /// If an Iroh node is not found in `iroh_dir`, a new one is created
    ///
    /// If an Iroh node is found in `iroh_dir` and `overwrite` is `false`, load it,
    /// otherwise truncate it
    pub async fn new(iroh_dir: &Path, overwrite: bool) -> Result<Self> {
        if overwrite {
            // remove old root dir if one existed before
            let _ = fs::remove_dir_all(iroh_dir);
        }
        // create root if not exists
        fs::create_dir_all(iroh_dir).await?;

        let iroh_node = iroh::node::Node::persistent(iroh_dir)
            .await?
            .spawn()
            .await?;

        let root = Root::load(iroh_dir).await?;

        let lis = Lis {
            iroh_node,
            iroh_dir: iroh_dir.to_path_buf(),
            rt: tokio::runtime::Handle::current(),
            root,
        };
        Ok(lis)
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
    async fn import_dir() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        let file_path = tmp_dir.path();
        let mut file = NamedTempFile::new_in(file_path).expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");
        let src_path = file.path();
        let dst_path = Path::new(file.path().file_name().unwrap());

        lis.import_file(src_path, dst_path)
            .await
            .expect("Could not import file"); // should succeed
        let get_content = lis.read(dst_path).await.expect("Could not get file"); // should succeed
        assert_eq!(get_content, content);
    }

    #[tokio::test]
    async fn import_file_twice() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");
        let binding = file.path().to_path_buf();
        let dst_path = Path::new(binding.file_name().unwrap());

        // import file twice
        lis.import_file(file.path(), dst_path)
            .await
            .expect("Could not import file"); // should succeed

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");

        lis.import_file(file.path(), dst_path)
            .await
            .expect("Could not import file");

        let get_content = lis.read(dst_path).await.expect("Could not get file"); // should succeed

        let files = lis.list(Path::new("/")).await.expect("Could not get file"); // should succeed

        assert_eq!(get_content, "Brian was here. Briefly. more"); // new content should be there
        assert_eq!(files.len(), 1); // there should only be one file
    }

    #[tokio::test]
    async fn import_file() {
        let tmp_dir = TempDir::new().expect("Could not create temp dir");
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).expect("Could not write to named temp file");

        let binding = file.path().to_path_buf();
        let dst_path = Path::new(binding.file_name().unwrap());

        lis.import_file(file.path(), dst_path)
            .await
            .expect("Could not import file");

        // but second time has more content
        let more_content = " more";
        write!(file, "{}", more_content).expect("Could not write to named temp file");
        lis.import_file(file.path(), dst_path)
            .await
            .expect("Could not import file");

        let get_content = lis.read(dst_path).await.expect("Could not get file"); // should succeed

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

        // create /1
        lis.mkdir(&Path::new("/1").to_path_buf(), None, None, None)
            .await
            .unwrap();
        assert_eq!(lis.list(Path::new("/")).await.unwrap().len(), 1);

        // create /1/2
        lis.mkdir(&Path::new("/1/2").to_path_buf(), None, None, None)
            .await
            .unwrap();
        assert_eq!(lis.list(Path::new("/1")).await.unwrap().len(), 1);

        // create /1/2/3
        lis.mkdir(&Path::new("/1/2/3").to_path_buf(), None, None, None)
            .await
            .unwrap();
        assert_eq!(lis.list(Path::new("/1/2")).await.unwrap().len(), 1);

        // add file /1/2/3/myfile.txt
        let src_path = file.path();
        let dst_path = Path::new("/1/2/3").join(file.path().file_name().unwrap());

        // import file twice
        lis.import_file(src_path, &dst_path)
            .await
            .expect("Could not import file");

        // check if file was created in path
        let files = lis.list(Path::new("/1/2/3")).await.unwrap(); // should succeed
        assert_eq!(files.len(), 1); // there should be exactly one file

        // retrieve content from the file
        let get_content = lis.read(&dst_path).await.unwrap(); // should succeed
        assert_eq!(get_content, "Brian was here. Briefly."); // new content should be there
    }

    #[tokio::test]
    async fn rmdir() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").unwrap();
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).unwrap();

        // create /1
        lis.mkdir(&Path::new("/1").to_path_buf(), None, None, None)
            .await
            .unwrap();
        assert_eq!(lis.list(Path::new("/")).await.unwrap().len(), 1);

        // create /1/2
        lis.mkdir(&Path::new("/1/2").to_path_buf(), None, None, None)
            .await
            .unwrap();
        assert_eq!(lis.list(Path::new("/1")).await.unwrap().len(), 1);

        // rmdir /1/2
        lis.rmdir(&Path::new("/1/2").to_path_buf()).await.unwrap();
        assert_eq!(lis.list(Path::new("/1")).await.unwrap().len(), 0);

        // rmdir /1
        lis.rmdir(&Path::new("/1").to_path_buf()).await.unwrap();
        assert_eq!(lis.list(Path::new("/")).await.unwrap().len(), 0);

        // rmdir / (should fail)
        let should_be_err = lis.rmdir(&Path::new("/").to_path_buf()).await;
        assert!(should_be_err.is_err());
        if let Err(e) = should_be_err {
            assert_eq!(e.to_string(), "Cannot delete root dir");
        }
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
        let get_content = lis.read(file_path).await.unwrap();
        assert_eq!(get_content, "null");
    }

    #[tokio::test]
    async fn write() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create empty file (touch) in lis
        let file_path = Path::new("/myfile.txt");
        lis.touch(&file_path.to_path_buf(), None, None, None)
            .await
            .unwrap();

        // retrieve content from the file (should be b"null")
        assert_eq!(lis.read(file_path).await.unwrap(), "null");

        // write to file
        lis.write(file_path, b"new data", 0).await.unwrap();

        // ensure new content is there
        assert_eq!(lis.read(file_path).await.unwrap(), "new data");
    }

    #[tokio::test]
    async fn remove() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create empty file (touch) in lis
        let file_path = Path::new("/myfile.txt");
        lis.touch(&file_path.to_path_buf(), None, None, None)
            .await
            .unwrap();

        // retrieve content from the file (should be b"null")
        assert_eq!(lis.read(file_path).await.unwrap(), "null");

        // remove file
        let _ = lis.remove(file_path).await.unwrap();

        // ensure file no longer exists
        assert_eq!(lis.list(Path::new("/")).await.unwrap().len(), 0);
    }
}

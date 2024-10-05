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
use objects::{dir::LisDir, file::LisFile, LisRoot, ObjectType};

pub struct Lis {
    pub iroh_node: Node<Store>,
    iroh_dir: PathBuf,
    pub rt: tokio::runtime::Handle,
    root: LisRoot,
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

        let root = LisRoot::load(iroh_dir).await?;

        let lis = Lis {
            iroh_node,
            iroh_dir: iroh_dir.to_path_buf(),
            rt: tokio::runtime::Handle::current(),
            root,
        };
        Ok(lis)
    }

    // pub async fn create_file(&mut self, full_path: &Path) -> Result<()> {
    //     match self.root.find(full_path).await {
    //         Some(ObjectType::File(_file)) => return Err(anyhow!("File exists")),
    //         Some(ObjectType::Dir(_dir)) => return Err(anyhow!("Is a directory")),
    //         None => {}
    //     }
    //     // new LisFile
    //     // find Dir where file is in
    //     // if file already exists, error
    //     // put file in dir
    //     Ok(())
    // }

    pub async fn create_dir(&mut self, full_path: &Path) -> Result<()> {
        match self.root.find(full_path).await? {
            Some(ObjectType::File(_file)) => return Err(anyhow!("Path is a file")),
            Some(ObjectType::Dir(_dir)) => return Err(anyhow!("Directory exists")),
            None => {}
        }

        let mut parent_dir = match self
            .root
            .find(full_path.parent().ok_or(anyhow!("No parent for dir"))?)
            .await?
            .ok_or(anyhow!("No parent for dir"))?
        {
            ObjectType::File(_file) => return Err(anyhow!("Parent is a file")),
            ObjectType::Dir(dir) => dir,
        };

        let dir = LisDir::new(self.iroh_node.clone()).await?;
        parent_dir.put_dir(dir).await?;
        Ok(())
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
    async fn mkdir() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // Create a file inside of `env::temp_dir()`.
        let mut file = NamedTempFile::new_in("/tmp/").unwrap();
        let content = "Brian was here. Briefly.";
        write!(file, "{}", content).unwrap();

        // create /1
        lis.create_dir(&Path::new("/1")).await.unwrap();
        assert_eq!(lis.list(Path::new("/")).await.unwrap().len(), 1);

        // create /1/2
        lis.create_dir(&Path::new("/1/2")).await.unwrap();
        assert_eq!(lis.list(Path::new("/1")).await.unwrap().len(), 1);

        // create /1/2/3
        lis.create_dir(&Path::new("/1/2/3")).await.unwrap();
        assert_eq!(lis.list(Path::new("/1/2")).await.unwrap().len(), 1);
    }
}

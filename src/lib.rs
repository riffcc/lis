use tokio::fs;

pub mod prelude;
use prelude::*;

mod util;
use util::get_relative_path;

mod cli;
pub use cli::{Cli, Commands};

mod objects;
use objects::{dir::LisDir, file::LisFile, LisRoot, ObjectType};

pub struct Lis {
    pub iroh_node: Node<Store>,
    _iroh_dir: PathBuf,
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
            let _ = fs::remove_dir_all(iroh_dir).await;
        }
        // create root if not exists
        fs::create_dir_all(iroh_dir).await?;

        let iroh_node = iroh::node::Node::persistent(iroh_dir)
            .await?
            .spawn()
            .await?;

        let root = LisRoot::load(iroh_node.client(), iroh_dir).await?;

        let lis = Lis {
            iroh_node,
            _iroh_dir: iroh_dir.to_path_buf(),
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

    // create /1/2
    pub async fn create_dir(&mut self, full_path: &Path) -> Result<()> {
        match self.root.find(self.iroh_node.client(), full_path).await? {
            Some(ObjectType::File(_file)) => return Err(anyhow!("Path is a file")),
            Some(ObjectType::Dir(_dir)) => return Err(anyhow!("Directory exists")),
            None => {}
        }

        let parent_path = full_path.parent().ok_or(anyhow!("No parent for dir"))?;
        let mut parent_dir = match self
            .root
            .find(self.iroh_node.client(), parent_path)
            .await?
            .ok_or(anyhow!(
                "Could not find doc for parent dir {}",
                parent_path.display()
            ))? {
            ObjectType::File(_file) => return Err(anyhow!("Parent is a file")),
            ObjectType::Dir(dir) => dir,
        };

        let relpath = get_relative_path(full_path, parent_path)
            .ok_or(anyhow!("Could not find relative path"))?;

        debug!(
            "adding {}; parent={}({}); relpath={}",
            full_path.display(),
            parent_path.display(),
            parent_dir.doc_id,
            relpath.display()
        );

        let (dir, _dir_id) = LisDir::new(self.iroh_node.client()).await?;
        parent_dir
            .put_dir(self.iroh_node.client(), &relpath, dir)
            .await?;
        Ok(())
    }

    pub async fn list(&self, full_path: &Path) -> Result<Vec<PathBuf>> {
        let dir: LisDir = match self.root.find(self.iroh_node.client(), full_path).await? {
            Some(ObjectType::Dir(dir)) => dir,
            Some(ObjectType::File(_file)) => return Err(anyhow!("Path is a file")),
            None => return Err(anyhow!("Path not found")),
        };

        dir.entries(&self.iroh_node.client()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::io::Write;
    use tempfile::TempDir;

    async fn setup_lis(tmp_dir: &TempDir) -> Lis {
        let root = PathBuf::from(tmp_dir.path());
        let overwrite = true;
        Lis::new(&root, overwrite)
            .await
            .expect("Could not create new Lis node")
    }
    #[tokio::test]
    async fn test_create_dir() {
        let tmp_dir = TempDir::new().unwrap();
        let mut lis = setup_lis(&tmp_dir).await;

        // create /1
        lis.create_dir(&Path::new("/1")).await.unwrap();
        let entries = lis.list(Path::new("/")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], Path::new("1"));

        // create /1/2
        lis.create_dir(&Path::new("/1/2")).await.unwrap();
        let entries = lis.list(Path::new("/1")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], Path::new("2"));

        // create /1/2/3
        lis.create_dir(&Path::new("/1/2/3")).await.unwrap();
        let entries = lis.list(Path::new("/1/2")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], Path::new("3"));

        // create /1/2/3 (again, error: dir exists)
        assert!(lis.create_dir(&Path::new("/1/2/3")).await.is_err());
    }
}

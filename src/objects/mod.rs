use crate::prelude::*;

mod children;
use children::Children;

pub mod metadata;
use metadata::Metadata;

pub mod file;
use file::LisFile;

pub mod dir;
use dir::LisDir;

pub mod inode;
use inode::InodeMap;

pub enum ObjectType {
    File(LisFile),
    Dir(LisDir),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LisRoot {
    dir: LisDir,
    inode_map: InodeMap,
}

impl LisRoot {
    pub async fn load(node: &Iroh, iroh_dir: &Path) -> Result<Self> {
        let root_path = iroh_dir.join(Path::new(".ROOT"));
        match root_path.exists() {
            true => {
                // load root from file
                let mut file = File::open(root_path).await?;
                let mut content = String::new();
                file.read_to_string(&mut content).await?;
                let root: Self = serde_json::from_str(&content).unwrap();

                Ok(root)
            }
            false => {
                let (root_dir, _root_dir_id) = LisDir::new(&node.clone()).await?;

                let root = Self {
                    dir: root_dir,
                    inode_map: InodeMap::new(&node.clone()).await?,
                };

                // create .ROOT file and write root Dir doc id and InodeMap doc id to it
                let mut file = File::create(root_path).await?;
                file.write_all(serde_json::to_string(&root)?.as_bytes())
                    .await?;

                Ok(root)
            }
        }
    }

    pub async fn find(&self, node: &Iroh, full_path: &Path) -> Result<Option<ObjectType>> {
        if full_path == Path::new("/") {
            return Ok(Some(ObjectType::Dir(self.dir.clone())));
        };

        // remove root from path
        let full_path_without_root: PathBuf = if !full_path.has_root() {
            return Err(anyhow!("Path is not absolute (no root found)"));
        } else {
            // remove root
            full_path.iter().skip(1).collect()
        };
        debug!(
            "path for find (no root): {}",
            full_path_without_root.display()
        );

        self.dir.find(node, &full_path_without_root).await
    }
}

pub trait FromNamespaceId: Sized {
    // required methods
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self>;
}

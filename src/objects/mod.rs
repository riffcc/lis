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
    pub async fn load(iroh_dir: &Path) -> Result<Self> {
        let root_path = iroh_dir.join(Path::new(".ROOT"));
        match root_path.exists() {
            false => {
                let node = Node::persistent(iroh_dir).await?.spawn().await?;

                let root = Self {
                    dir: LisDir::new(node.clone()).await?,
                    inode_map: InodeMap::new(node).await?,
                };

                // create .ROOT file and write root Dir doc id and InodeMap doc id to it
                let mut file = File::create(root_path).await?;
                file.write_all(serde_json::to_string(&root)?.as_bytes())
                    .await?;

                Ok(root)
            }
            true => {
                // load root from file
                let mut file = File::open(root_path).await?;
                let mut content = String::new();
                file.read_to_string(&mut content).await?;
                let root: Self = serde_json::from_str(&content).unwrap();

                Ok(root)
            }
        }
    }

    pub async fn find(&self, full_path: &Path) -> Result<Option<ObjectType>> {
        self.dir.find(full_path).await
    }
}

pub trait FromNamespaceId {
    // required methods
    async fn from_namespace_id(node: Node<Store>, id: NamespaceId) -> Result<Self> {}
}

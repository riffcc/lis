use crate::prelude::*;

pub mod file;

pub mod dir;
use dir::Dir;

pub mod inode;
use inode::InodeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct Root {
    dir: Dir,
    inode_map: InodeMap,
}

impl Root {
    pub async fn load(iroh_dir: &Path) -> Result<Self> {
        let root_path = iroh_dir.join(Path::new(".ROOT"));
        match root_path.exists() {
            false => {
                let node = Node::persistent(iroh_dir).await?.spawn().await?;

                let root = Root {
                    dir: Dir::new(node.clone(), Path::new("/")).await?,
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
                let root: Root = serde_json::from_str(&content).unwrap();

                Ok(root)
            }
        }
    }
}

use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Dir {}

impl Dir {
    pub async fn new(_node: Node<iroh::blobs::store::fs::Store>, _path: &Path) -> Result<Self> {
        Ok(Dir {})
    }
}

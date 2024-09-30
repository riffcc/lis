use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct InodeMap {}

impl InodeMap {
    pub async fn new(_node: Node<iroh::blobs::store::fs::Store>) -> Result<Self> {
        Ok(InodeMap {})
    }
}

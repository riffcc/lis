use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct InodeMap {}

impl InodeMap {
    pub async fn new(_node: &Iroh) -> Result<Self> {
        Ok(InodeMap {})
    }
}

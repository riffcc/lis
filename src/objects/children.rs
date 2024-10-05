use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Children {
    doc_id: NamespaceId,
}
impl Children {
    pub async fn new(node: Node<iroh::blobs::store::fs::Store>) -> Result<Self> {
        let doc = node.docs().create().await?;

        Ok(Self { doc_id: doc.id() })
    }
}

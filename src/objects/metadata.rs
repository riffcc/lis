use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    doc_id: NamespaceId,
    pub items: usize,
}
impl Metadata {
    pub async fn new(node: Node<iroh::blobs::store::fs::Store>) -> Result<Self> {
        let doc = node.docs().create().await?;

        Ok(Self {
            doc_id: doc.id(),
            items: 0,
        })
    }

    pub async fn save(&self) -> Result<()> {
        let mut doc = find_doc(self.node, self.doc_id).await?;
        doc["items"] = self.items;
    }
}

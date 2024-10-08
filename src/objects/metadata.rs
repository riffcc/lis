use bytes::Bytes;

use crate::{doc::LisDoc, objects::FromNamespaceId, prelude::*};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    doc: LisDoc,
    pub items: usize,
}
impl Metadata {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = LisDoc::new(&node.clone()).await?;

        // set type to "children"
        doc.set(node, Key::from(".type".to_string()), "metadata".into())
            .await?;
        doc.set(
            node,
            Key::from("items".to_string()),
            Bytes::copy_from_slice(&(0 as usize).to_ne_bytes()),
        )
        .await?;

        let id = doc.id();
        Ok((Self { doc, items: 0 }, id))
    }

    pub async fn save(&mut self, node: &Iroh) -> Result<()> {
        self.doc
            .set(
                node,
                Key::from("items".to_string()),
                Bytes::copy_from_slice(&self.items.to_ne_bytes()),
            )
            .await?;

        Ok(())
    }
}

impl FromNamespaceId for Metadata {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = LisDoc::load(&node, id).await?;

        // check type
        if doc.doc_type(&node).await? != DocType::MetadataDoc {
            return Err(anyhow!("Doc is not a metadata doc"));
        }

        let items_bytes = doc
            .get(node, Key::from("items".to_string()))
            .await?
            .ok_or(anyhow!("Could not find items key in doc"))?;

        let items: usize = usize::from_ne_bytes(
            items_bytes.as_ref()[..std::mem::size_of::<usize>()]
                .try_into()
                .unwrap(),
        );

        Ok(Self { doc, items })
    }
}

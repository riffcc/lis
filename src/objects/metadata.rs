use bytes::Bytes;

use crate::{objects::FromNamespaceId, prelude::*};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    doc_id: NamespaceId,
    pub items: usize,
}
impl Metadata {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = node.docs().create().await?;

        // set type to "metadata"
        doc.set_bytes(
            node.authors().default().await?,
            Key::from(".type".to_string()),
            Bytes::from("metadata".to_string()),
        )
        .await?;

        // set items to 0
        doc.set_bytes(
            node.authors().default().await?,
            Key::from("items".to_string()),
            Bytes::copy_from_slice(&(0 as usize).to_ne_bytes()),
        )
        .await?;

        Ok((
            Self {
                doc_id: doc.id(),
                items: 0,
            },
            doc.id(),
        ))
    }

    pub async fn save(&self, node: &Iroh) -> Result<()> {
        let doc = load_doc(&node, self.doc_id).await?;

        let key = Key::from("items".to_string());
        let value: Bytes = Bytes::copy_from_slice(&self.items.to_ne_bytes());

        doc.set_bytes(node.authors().default().await?, key, value)
            .await?;
        Ok(())
    }
}

impl FromNamespaceId for Metadata {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = load_doc(&node, id).await?;

        // check type
        if doc_type(&node, &doc).await? != DocType::MetadataDoc {
            return Err(anyhow!("Doc is not a metadata doc"));
        }

        let default_author = node.authors().default().await?;

        let items_bytes = doc
            .get_exact(default_author, Key::from("items".to_string()), false)
            .await?
            .ok_or(anyhow!("items entry not found"))?
            .content_bytes(&node.clone())
            .await?;

        let items: usize = usize::from_ne_bytes(
            items_bytes.as_ref()[..std::mem::size_of::<usize>()]
                .try_into()
                .unwrap(),
        );

        Ok(Self { doc_id: id, items })
    }
}

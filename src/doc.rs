use futures_lite::stream::StreamExt; // For collect

use crate::{objects::FromNamespaceId, prelude::*, util};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LisDoc {
    doc_id: NamespaceId,
}

impl LisDoc {
    pub async fn new(node: &Iroh) -> Result<Self> {
        let doc = node.docs().create().await?;

        Ok(Self { doc_id: doc.id() })
    }

    pub fn id(&self) -> NamespaceId {
        self.doc_id
    }

    pub async fn load(node: &Iroh, id: NamespaceId) -> Result<Self> {
        Self::from_namespace_id(node, id).await
    }

    pub async fn doc_type(&self, node: &Iroh) -> Result<DocType> {
        Ok(DocType::from(
            self.get(node, Key::from(".type".to_string()))
                .await?
                .ok_or(anyhow!("Could not find .type key in doc"))?,
        ))
    }

    pub async fn set(&self, node: &Iroh, key: Key, value: Bytes) -> Result<()> {
        self.iroh_doc(node)
            .await?
            .set_bytes(
                node.authors().default().await?,
                <util::Key as Into<Bytes>>::into(key),
                value,
            )
            .await?;
        Ok(())
    }

    pub async fn get(&self, node: &Iroh, key: Key) -> Result<Option<Bytes>> {
        match self
            .iroh_doc(node)
            .await?
            .get_exact(node.authors().default().await?, key, false)
            .await?
        {
            Some(entry) => Ok(Some(entry.content_bytes(&node.clone()).await?)),
            None => Ok(None),
        }
    }

    pub async fn entries(&self, node: &Iroh) -> Result<Vec<Result<Entry>>> {
        let query = Query::all().build();
        Ok(self
            .iroh_doc(node)
            .await?
            .get_many(query)
            .await?
            .collect::<Vec<_>>()
            .await)
    }

    async fn iroh_doc(&self, node: &Iroh) -> Result<Doc> {
        node.docs()
            .open(self.doc_id)
            .await?
            .ok_or(anyhow!("could not open iroh doc"))
    }
}

impl FromNamespaceId for LisDoc {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        Ok(Self { doc_id: id })
    }
}

#[derive(Debug, PartialEq)]
pub enum DocType {
    DirDoc,
    ChildrenDoc,
    MetadataDoc,
    ChunksDoc,
    FileDoc,
    RootDoc,
    Unknown,
}

impl From<Bytes> for DocType {
    fn from(bytes: Bytes) -> Self {
        match String::from_utf8(bytes.to_vec()).unwrap().as_ref() {
            "root" => DocType::RootDoc,
            "dir" => DocType::DirDoc,
            "children" => DocType::ChildrenDoc,
            "metadata" => DocType::MetadataDoc,
            "file" => DocType::FileDoc,
            "chunks" => DocType::ChunksDoc,
            _ => DocType::Unknown,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_doc_type() {
        let node = iroh::node::Node::memory().spawn().await.unwrap();
        let doc = LisDoc::new(&node).await.unwrap();

        // set type to "children"
        doc.set(&node, Key::from(".type".to_string()), "children".into())
            .await
            .unwrap();
        assert_eq!(doc.doc_type(&node).await.unwrap(), DocType::ChildrenDoc);
    }
}

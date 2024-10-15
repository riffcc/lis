use crate::{
    doc::LisDoc,
    objects::{FromNamespaceId, Metadata, ObjectType},
    prelude::*,
};

pub struct LisFile {
    doc: LisDoc,
    chunks: Chunks,
    metadata: Metadata,
}

impl LisFile {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let (chunks, chunks_id) = Chunks::new(&node.clone()).await?;
        let (metadata, metadata_id) = Metadata::new(&node.clone()).await?;

        let doc = LisDoc::new(node).await?;
        doc.set(
            node,
            Key::from(".metadata".to_string()),
            namespace_id_to_bytes(metadata_id),
        )
        .await?;
        doc.set(
            node,
            Key::from(".chunks".to_string()),
            namespace_id_to_bytes(chunks_id),
        )
        .await?;
        doc.set(node, Key::from(".type".to_string()), "file".into())
            .await?;

        let id = doc.id();
        Ok((
            Self {
                doc,
                chunks,
                metadata,
            },
            id,
        ))
    }
}

impl FromNamespaceId for LisFile {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = LisDoc::from_namespace_id(node, id).await?;

        // check type
        if doc.doc_type(node).await? != DocType::FileDoc {
            return Err(anyhow!("NamespaceId does not correspond to a file doc"));
        }

        let chunks_key = Key::from(".chunks".to_string());
        let chunks_id = bytes_to_namespace_id(
            doc.get(node, chunks_key)
                .await?
                .ok_or(anyhow!("Could not find chunks key in file doc"))?,
        )?;

        let metadata_key = Key::from(".metadata".to_string());
        let metadata_id = bytes_to_namespace_id(
            doc.get(node, metadata_key)
                .await?
                .ok_or(anyhow!("Could not find metadata key in file doc"))?,
        )?;

        Ok(Self {
            doc,
            chunks: Chunks::from_namespace_id(&node.clone(), chunks_id).await?,
            metadata: Metadata::from_namespace_id(&node.clone(), metadata_id).await?,
        })
    }
}

pub struct Chunks {
    doc: LisDoc,
}

impl Chunks {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = LisDoc::new(node).await?;
        doc.set(node, Key::from(".type".to_string()), "chunks".into())
            .await?;
        let id = doc.id();
        Ok((Self { doc }, id))
    }
}
impl FromNamespaceId for Chunks {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = LisDoc::from_namespace_id(node, id).await?;

        // check type
        if doc.doc_type(node).await? != DocType::ChunksDoc {
            return Err(anyhow!("NamespaceId does not correspond to a chunks doc"));
        }

        Ok(Self { doc })
    }
}

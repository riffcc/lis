use bytes::Bytes;

use crate::{
    doc::LisDoc,
    objects::{FromNamespaceId, ObjectType},
    prelude::*,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObjectAttributes {
    FileAttributes { chunks: usize, chunk_size: usize },
    DirAttributes { items: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    doc: LisDoc,
    pub kind: String,
    pub attrs: ObjectAttributes,
}

impl Metadata {
    pub async fn new(node: &Iroh, object_type: ObjectType) -> Result<(Self, NamespaceId)> {
        let doc = LisDoc::new(&node.clone()).await?;
        let id = doc.id();

        // set type to "children"
        doc.set(node, Key::from(".type".to_string()), "metadata".into())
            .await?;

        let metadata = match object_type {
            ObjectType::File => {
                let attrs = ObjectAttributes::FileAttributes {
                    chunks: 0,
                    chunk_size: DEFAULT_CHUNK_SIZE,
                };
                Self {
                    doc,
                    kind: "file".to_string(),
                    attrs,
                }
            }
            ObjectType::Dir => {
                let attrs = ObjectAttributes::DirAttributes { items: 0 };
                Self {
                    doc,
                    kind: "dir".to_string(),
                    attrs,
                }
            }
        };
        metadata.save(node).await?;

        Ok((metadata, id))
    }

    pub async fn save(&self, node: &Iroh) -> Result<()> {
        self.doc
            .set(
                node,
                Key::from("kind".to_string()),
                self.kind.clone().into(),
            )
            .await?;

        match self.attrs {
            ObjectAttributes::DirAttributes { items } => {
                self.doc
                    .set(
                        node,
                        Key::from("items".to_string()),
                        Bytes::copy_from_slice(&items.to_ne_bytes()),
                    )
                    .await?;
            }
            ObjectAttributes::FileAttributes { chunks, chunk_size } => {
                self.doc
                    .set(
                        node,
                        Key::from("chunks".to_string()),
                        Bytes::copy_from_slice(&chunks.to_ne_bytes()),
                    )
                    .await?;
                self.doc
                    .set(
                        node,
                        Key::from("chunk_size".to_string()),
                        Bytes::copy_from_slice(&chunk_size.to_ne_bytes()),
                    )
                    .await?;
            }
        }

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

        let kind = String::from_utf8(
            doc.get(node, Key::from("kind".to_string()))
                .await?
                .ok_or(anyhow!("Could not find kind key in doc"))?
                .to_vec(),
        )?;

        let attrs = match kind.as_ref() {
            "file" => {
                let chunk_size_bytes = doc
                    .get(node, Key::from("chunk_size".to_string()))
                    .await?
                    .ok_or(anyhow!("Could not find chunk_size key in doc"))?;
                let chunk_size: usize = usize::from_ne_bytes(
                    chunk_size_bytes.as_ref()[..std::mem::size_of::<usize>()].try_into()?,
                );
                let chunks_bytes = doc
                    .get(node, Key::from("chunks".to_string()))
                    .await?
                    .ok_or(anyhow!("Could not find chunks key in doc"))?;
                let chunks: usize = usize::from_ne_bytes(
                    chunks_bytes.as_ref()[..std::mem::size_of::<usize>()].try_into()?,
                );
                ObjectAttributes::FileAttributes { chunks, chunk_size }
            }
            "dir" => {
                let items_bytes = doc
                    .get(node, Key::from("items".to_string()))
                    .await?
                    .ok_or(anyhow!("Could not find items key in doc"))?;
                let items: usize = usize::from_ne_bytes(
                    items_bytes.as_ref()[..std::mem::size_of::<usize>()].try_into()?,
                );
                ObjectAttributes::DirAttributes { items }
            }
            _ => return Err(anyhow!("Unknown doc type, expected 'file' or 'dir'.")),
        };

        Ok(Self { doc, kind, attrs })
    }
}

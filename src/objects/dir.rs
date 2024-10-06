use std::path::Component;

use iroh::docs::NamespaceId;

use crate::{
    objects::{Children, FromNamespaceId, LisFile, Metadata, ObjectType},
    prelude::*,
};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LisDir {
    pub doc_id: NamespaceId,
    children: Children,
    metadata: Metadata,
}

struct DirEntry {
    entry_type: ObjectType,
    key: Key,
}

impl LisDir {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let (children, children_id) = Children::new(&node.clone()).await?;
        let (metadata, metadata_id) = Metadata::new(&node.clone()).await?;

        let doc = node.docs().create().await?;
        doc.set_bytes(
            node.authors().default().await?,
            Key::from("metadata".to_string()),
            namespace_id_to_bytes(metadata_id),
        )
        .await?;
        doc.set_bytes(
            node.authors().default().await?,
            Key::from("children".to_string()),
            namespace_id_to_bytes(children_id),
        )
        .await?;

        // set type to "dir"
        doc.set_bytes(
            node.authors().default().await?,
            Key::from(".type".to_string()),
            Bytes::from("dir".to_string()),
        )
        .await?;

        Ok((
            Self {
                doc_id: doc.id(),
                children,
                metadata,
            },
            doc.id(),
        ))
    }

    pub async fn get(&self, node: &Iroh, path: PathBuf) -> Result<Option<ObjectType>> {
        self.children.get(node, path).await
    }

    pub async fn find(&self, node: &Iroh, path: &Path) -> Result<Option<ObjectType>> {
        let mut cur_dir = LisDir::from_namespace_id(&node.clone(), self.doc_id).await?;
        for component in path.components() {
            match component {
                Component::Normal(osstr) => {
                    let cur_path = Path::new(osstr);
                    debug!("cur_path={}", cur_path.display());
                    match cur_dir.get(&node.clone(), cur_path.into()).await? {
                        Some(ObjectType::Dir(next_dir)) => {
                            cur_dir = next_dir;
                        }
                        Some(ObjectType::File(file)) => return Ok(Some(ObjectType::File(file))),
                        None => return Ok(None),
                    }
                }
                _ => return Err(anyhow!("Invalid component in path")),
            }
        }
        Ok(Some(ObjectType::Dir(cur_dir)))
    }

    pub async fn put_dir(&mut self, node: &Iroh, path: &Path, dir: LisDir) -> Result<()> {
        debug!("putting {}", path.display());
        self.children.put(node, path.to_path_buf(), dir).await?;
        self.metadata.items += 1;
        self.metadata.save(&node).await
    }

    pub async fn entries(&self, node: &Iroh) -> Result<Vec<PathBuf>> {
        self.children.entries(node).await
    }
}

impl FromNamespaceId for LisDir {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = load_doc(&node, id).await?;

        // check type
        if doc_type(&node, &doc).await? != DocType::DirDoc {
            return Err(anyhow!("NamespaceId does not correspond to a dir doc"));
        }

        let default_author = node.authors().default().await?;

        let children_key = Key::from("children".to_string());
        let children_id = bytes_to_namespace_id(
            doc.get_exact(default_author, children_key, false)
                .await?
                .ok_or(anyhow!("children not found"))?
                .content_bytes(&node.clone())
                .await?,
        )?;

        let metadata_key = Key::from("metadata".to_string());
        let metadata_id = bytes_to_namespace_id(
            doc.get_exact(default_author, metadata_key, false)
                .await?
                .ok_or(anyhow!("metadata not found"))?
                .content_bytes(&node.clone())
                .await?,
        )?;

        Ok(Self {
            doc_id: id,
            children: Children::from_namespace_id(&node, children_id).await?,
            metadata: Metadata::from_namespace_id(&node, metadata_id).await?,
        })
    }
}

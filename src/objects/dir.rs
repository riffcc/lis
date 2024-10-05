use std::default;

use iroh::{blobs::protocol::NonEmptyRequestRangeSpecIter, docs::NamespaceId};

use crate::{
    objects::{Children, FromNamespaceId, LisFile, Metadata, ObjectType},
    prelude::*,
    util::{load_doc, namespace_id_to_bytes, split_path, DocId, DocType, Key},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LisDir {
    doc_id: NamespaceId,
    children: Children,
    metadata: Metadata,
}

struct DirEntry {
    entry_type: ObjectType,
    key: Key,
}

impl LisDir {
    pub async fn new(node: Node<Store>) -> Result<(Self, NamespaceId)> {
        let (children, children_id) = Children::new(node.clone()).await?;
        let (metadata, metadata_id) = Metadata::new(node.clone()).await?;

        let metadata_key = Key::from("metadata".to_string());
        let children_key = Key::from("children".to_string());

        let doc = node.docs().create().await?;
        // doc["metadata"] = metadata_id
        doc.set_bytes(
            node.authors().default().await?,
            metadata_key,
            namespace_id_to_bytes(metadata_id),
        )
        .await?;
        // doc["children"] = children_id
        doc.set_bytes(
            node.authors().default().await?,
            children_key,
            namespace_id_to_bytes(children_id),
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

    pub async fn get(&self, node: Node<Store>, key_str: String) -> Result<Option<ObjectType>> {
        if path.components().len() != 1 {
            return Err(anyhow!("Wrong path type to get"));
        }

        let doc = load_doc(node, self.doc_id).await?;

        let key = Key::from(key_str);

        let query = Query::key_exact(key);
        let entry = doc
            .get_one(query)
            .await?
            .ok_or_else(|| anyhow!("entry not found"))?;

        let content = entry.content_bytes(node.client()).await?;
    }

    pub async fn find(&self, path: &Path) -> Result<Option<ObjectType>> {
        match split_path(path) {
            Some((next, None)) => {
                // base case: either here or not present
                if let Some(entry) = self.children.get(node, next).await? {
                    Ok(Some(entry))
                } else {
                    Ok(None)
                }
            }
            Some((next, Some(rest))) => {
                // move on to next doc
                if let Some(entry) = self.children.get(node, next).await? {
                    let next_doc_id = DocId::from(entry);
                    let next_doc = load_doc(node, next_doc_id).await?;
                    next_doc.find(rest)
                } else {
                    Ok(None)
                }
            }
            None => Err(anyhow!("Unexpected error while looking for object")),
        }
    }

    pub async fn put_dir(&mut self, dir: LisDir) -> Result<()> {
        self.metadata.items += 1;
        self.metadata.save().await?;
        self.children.put(dir).await
    }
}

impl FromNamespaceId for LisDir {
    async fn from_namespace_id(node: Node<Store>, id: NamespaceId) -> Result<Self> {
        let doc = load_doc(node, id).await?;

        // check type
        if doc_type(node, doc) != DocType::DirDoc {
            return Err(anyhow!("NamespaceId does not correspond to a dir doc"));
        }

        let default_author = node.authors().default().await?;
        let children_key = Key::from("children".to_string());
        let metadata_key = Key::from("metadata".to_string());

        let children_id = bytes_to_namespace_id(
            doc.get_exact(default_author, children_key, false)
                .await?
                .ok_or(anyhow!("children entry not found"))?,
        );

        let metadata_id = bytes_to_namespace_id(
            doc.get_exact(default_author, children_key, false)
                .await?
                .ok_or(anyhow!("metadata not found"))?,
        );

        Ok(Self {
            doc_id: id,
            children: Children::from_namespace_id(node.clone(), children_id).await?,
            metadata: Metadata::from_namespace_id(node.clone(), metadata_id).await?,
        })
    }
}

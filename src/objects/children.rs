use crate::{
    objects::{dir::LisDir, FromNamespaceId, ObjectType},
    prelude::*,
};
use futures_lite::stream::StreamExt; // For collect

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Children {
    doc_id: NamespaceId,
}
impl Children {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = node.docs().create().await?;

        // set type to "children"
        doc.set_bytes(
            node.authors().default().await?,
            Key::from(".type".to_string()),
            Bytes::from("children".to_string()),
        )
        .await?;

        Ok((Self { doc_id: doc.id() }, doc.id()))
    }

    pub async fn get(&self, node: &Iroh, path: PathBuf) -> Result<Option<ObjectType>> {
        if path.components().count() != 1 {
            return Err(anyhow!("Incorrect path, more than one component"));
        }
        let doc = load_doc(&node, self.doc_id).await?;

        let key = Key::from(path);

        let query = Query::key_exact(key);
        let entry = match doc.get_one(query).await? {
            Some(entry) => entry,
            None => return Ok(None),
        };

        let content = entry.content_bytes(&node.clone()).await?;
        // bytes to doc id
        let doc_id = bytes_to_namespace_id(content)?;
        // lisdir or file from doc id
        // TODO: support files
        let object = LisDir::from_namespace_id(node, doc_id).await?;
        Ok(Some(ObjectType::Dir(object)))
    }

    pub async fn put(&self, node: &Iroh, path: PathBuf, object: LisDir) -> Result<()> {
        // TODO: support dir or file in object
        if path.components().count() != 1 {
            return Err(anyhow!("Path has more than one component"));
        }
        let key = Key::from(path);
        let value = namespace_id_to_bytes(object.doc_id);

        let doc = load_doc(&node, self.doc_id).await?;
        doc.set_bytes(node.authors().default().await?, key, value)
            .await?;
        Ok(())
    }
    pub async fn entries(&self, node: &Iroh) -> Result<Vec<PathBuf>> {
        let doc = load_doc(node, self.doc_id).await?;

        let query = Query::all().build();
        let entries = doc.get_many(query).await?.collect::<Vec<_>>().await;

        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry?;
            let key = Key::from(entry.key());
            let path: PathBuf = key.into();

            // ignore files or dirs that start with . (e.g. .type)
            if !path
                .to_str()
                .ok_or(anyhow!("could not convert path to string"))?
                .starts_with(".")
            {
                debug!("path: {} (does not start with .)", path.display());
                paths.push(path);
            }
        }
        Ok(paths)
    }
}
impl FromNamespaceId for Children {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = load_doc(&node, id).await?;

        // check type
        if doc_type(node, &doc).await? != DocType::ChildrenDoc {
            return Err(anyhow!("Doc is not a children doc"));
        }

        Ok(Self { doc_id: id })
    }
}

use crate::{
    doc::LisDoc,
    objects::{dir::LisDir, FromNamespaceId, ObjectType},
    prelude::*,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Children {
    doc: LisDoc,
}
impl Children {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = LisDoc::new(&node.clone()).await?;

        // set type to "children"
        doc.set(node, Key::from(".type".to_string()), "children".into())
            .await?;

        let id = doc.id();
        Ok((Self { doc }, id))
    }

    pub async fn get(&self, node: &Iroh, path: PathBuf) -> Result<Option<ObjectType>> {
        if path.components().count() != 1 {
            return Err(anyhow!("Incorrect path, more than one component"));
        }
        let key = Key::from(path);
        let content = match self.doc.get(&node.clone(), key).await? {
            Some(content) => content,
            None => return Ok(None),
        };
        let doc_id = bytes_to_namespace_id(content)?;
        // lisdir or file from doc id
        // TODO: support files
        Ok(Some(ObjectType::Dir(
            LisDir::from_namespace_id(node, doc_id).await?, // TODO: from_namespace_id for ObjectType
        )))
    }

    pub async fn put(&self, node: &Iroh, path: PathBuf, object_id: NamespaceId) -> Result<()> {
        // TODO: support dir or file in object
        if path.components().count() != 1 {
            return Err(anyhow!("Path has more than one component"));
        }
        let key = Key::from(path);
        let value = namespace_id_to_bytes(object_id);

        self.doc.set(node, key, value).await?;
        Ok(())
    }
    pub async fn entries(&self, node: &Iroh) -> Result<Vec<PathBuf>> {
        let entries = self.doc.entries(node).await?;

        let mut paths = Vec::new();
        for entry in entries {
            let key = Key::from(entry?.key());
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
        let doc = LisDoc::load(&node, id).await?;

        // check type
        if doc.doc_type(&node).await? != DocType::ChildrenDoc {
            return Err(anyhow!("Doc is not a children doc"));
        }

        Ok(Self { doc })
    }
}

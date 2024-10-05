use crate::prelude::*;
use bytes::Bytes;
use iroh::docs::NamespaceId;

pub enum DocType {
    DirDoc,
    ChildrenDoc,
    MetadataDoc,
    FileChunkDoc,
    FileDoc,
    RootDoc,
    Unkown,
}

pub struct Key(Bytes);

impl From<PathBuf> for Key {
    fn from(path: PathBuf) -> Self {
        // Convert the PathBuf to a byte slice
        let path_bytes = path.to_string_lossy().as_bytes().to_vec();

        // Convert the byte slice to Bytes
        let key_bytes = Bytes::from(path_bytes);

        Key(key_bytes)
    }
}

impl From<String> for Key {
    fn from(s: String) -> Self {
        Key(Bytes::from(s.into_bytes()))
    }
}

impl From<NamespaceId> for Key {
    fn from(id: NamespaceId) -> Self {
        Key(namespace_id_to_bytes(id))
    }
}

pub async fn load_doc(node: Node<Store>, doc_id: NamespaceId) -> Result<Doc> {
    node.docs()
        .open(doc_id)
        .await?
        .ok_or(anyhow!("no files doc found"))
}

/// Converts NamespaceId to Bytes
pub fn namespace_id_to_bytes(id: NamespaceId) -> Bytes {
    let byte_vec = id.to_bytes().to_vec();
    Bytes::from(byte_vec)
}

/// Converts Bytes to NamespaceId
pub fn bytes_to_namespace_id(bytes: Bytes) -> Result<NamespaceId> {
    let array: &[u8; 32] = bytes.as_ref().try_into()?;
    Ok(array.into())
}

pub async fn doc_type(node: Node<Store>, doc: Doc) -> Result<DocType> {
    let bytes: Bytes = match doc
        .get_exact(
            node.authors().default().await?,
            Key::from("type".to_string()),
            false,
        )
        .await?
    {
        Some(entry) => entry.content_bytes(node.client()).await?.to_vec(),
        None => Err(anyhow!("type key not found in doc")),
    };

    Ok(match String::from_utf8(bytes)?.as_ref() {
        "root" => DocType::RootDoc,
        "dir" => DocType::DirDoc,
        "children" => DocType::ChildrenDoc,
        "metadata" => DocType::MetadataDoc,
        "file" => DocType::FileDoc,
        "fileChunk" => DocType::FileChunkDoc,
        _ => DocType::Unkown,
    })
}

pub fn split_path(path: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut components = path.components();

    let next = components.next()?.as_os_str();
    let next = PathBuf::from(next);

    let rest = components.as_path().to_path_buf();

    if rest.as_os_str().is_empty() {
        Some((next, None))
    } else {
        Some((next, Some(rest)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bytes_to_namespace_id() {
        let node = iroh::node::Node::memory().spawn().await.unwrap();
        let doc = node.docs().create().await.unwrap();
        let bytes = namespace_id_to_bytes(doc.id());
        let id = bytes_to_namespace_id(bytes).unwrap();
        assert_eq!(doc.id(), id);
    }
}

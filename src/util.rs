use std::convert::Into;
use std::str;

use iroh::docs::NamespaceId;

use crate::doc::LisDoc;
use crate::prelude::*;

pub struct Key(Bytes);

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Into<Bytes> for Key {
    fn into(self) -> Bytes {
        self.0
    }
}

impl Into<PathBuf> for Key {
    fn into(self) -> PathBuf {
        let key_str = str::from_utf8(&self.0).expect("Invalid UTF-8 in key");
        PathBuf::from(key_str)
    }
}

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

impl From<&[u8]> for Key {
    fn from(k: &[u8]) -> Self {
        Key(Bytes::copy_from_slice(k))
    }
}

impl From<NamespaceId> for Key {
    fn from(id: NamespaceId) -> Self {
        Key(namespace_id_to_bytes(id))
    }
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

pub fn get_relative_path(path: &Path, parent: &Path) -> Option<PathBuf> {
    // Strip the parent from the path
    if let Ok(relative_path) = path.strip_prefix(&parent) {
        // Return the remaining path as a PathBuf
        Some(relative_path.to_path_buf())
    } else {
        None // Return None if the parent is not a prefix of the path
    }
}

pub fn split_path(path: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut components = path.components();

    let next = components.next()?.as_os_str();
    let next = PathBuf::from(next);

    let rest = components.as_path().to_path_buf();

    debug!(
        "split_path: next={}, rest={}",
        next.display(),
        rest.display()
    );
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

    #[tokio::test]
    async fn test_get_relative_path() {
        assert_eq!(
            get_relative_path(Path::new("/hey/there"), Path::new("/hey")),
            Some(Path::new("there").to_path_buf())
        );
        assert_eq!(
            get_relative_path(Path::new("/this"), Path::new("/")),
            Some(Path::new("this").to_path_buf())
        );
        assert_eq!(
            get_relative_path(Path::new("/a/b/c"), Path::new("/d")),
            None,
        );
    }
}

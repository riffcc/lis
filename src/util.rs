use anyhow::{anyhow, Result};
use bytes::Bytes;
use iroh::{docs::NamespaceId, util::fs::path_to_key};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Converts NamespaceId to Bytes
pub fn namespaceid_to_bytes(id: NamespaceId) -> Bytes {
    let byte_vec = id.to_bytes().to_vec();
    Bytes::from(byte_vec)
}

/// Converts Bytes to NamespaceId
pub fn bytes_to_namespaceid(bytes: Bytes) -> Result<NamespaceId> {
    let array: &[u8; 32] = bytes.as_ref().try_into()?;
    Ok(array.into())
}

/// Generates a canonicalized key derived from `path` given a node's `root` dir path
pub fn key_from_file(root: &Path, path: &Path) -> Result<Bytes> {
    // Key is self.root + / + filename
    let mut prefix = root
        .as_os_str()
        .to_owned()
        .into_string()
        .expect("Could not make file path into string");
    prefix.push('/');

    let root: PathBuf = path
        .parent()
        .ok_or(anyhow!("Could not find parent for file"))?
        .into();

    // src_path = /os/path/filename.txt
    // prefix = /path/to/iroh/node
    // root = /os/path/
    // key = /path/to/iroh/node/filename.txt
    path_to_key(path, Some(prefix), Some(root))
}

pub fn key_to_string(key: Bytes) -> Result<String> {
    let key_str = std::str::from_utf8(key.as_ref())?;
    Ok(key_str.to_string())
}

pub fn get_paths_in_dir(dir_path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    // Read the directory contents
    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();
        paths.append(&mut if path.is_file() {
            vec![path]
        } else if path.is_dir() {
            get_paths_in_dir(&path)?
        } else {
            anyhow::bail!("{} has an unsupported path type", path.display());
        });
    }

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bytes_to_namespaceid() {
        let node = iroh::node::Node::memory().spawn().await.unwrap();
        let doc = node.docs().create().await.unwrap();
        let bytes = namespaceid_to_bytes(doc.id());
        let id = bytes_to_namespaceid(bytes).unwrap();
        assert_eq!(doc.id(), id);
    }
}

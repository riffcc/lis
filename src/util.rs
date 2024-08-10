use anyhow::{anyhow, Result};
use bytes::Bytes;
use iroh::util::fs::path_to_key;
use std::path::{Path, PathBuf};

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

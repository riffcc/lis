use anyhow::{anyhow, Result};
use bytes::Bytes;
use iroh::util::fs::path_to_key;
use std::{
    fs,
    path::{Path, PathBuf},
};

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

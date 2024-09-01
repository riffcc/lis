use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::fuse::{path_kind, InodeAttributes};
use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Object {
    pub path: PathBuf,
    pub attr: InodeAttributes,
}

impl Object {
    pub fn new(path: &Path, inode: u64) -> Result<Self> {
        let ts = SystemTime::now();
        let size = fs::metadata(path)?.len();
        let attr = InodeAttributes {
            inode,
            open_file_handles: 0,
            size,
            last_accessed: ts,
            last_modified: ts,
            last_metadata_changed: ts,
            kind: path_kind(path)?.into(),
            mode: 0o755,
            hardlinks: 1,
            uid: 0,
            gid: 0,
            xattrs: Default::default(),
        };
        Ok(Object {
            path: path.to_path_buf(),
            attr,
        })
    }
}

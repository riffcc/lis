use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::fuse::{FileKind, InodeAttributes};
use crate::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Object {
    /// Absolute path to object
    pub path: PathBuf,
    pub attr: InodeAttributes,
}

impl Object {
    pub fn new(path: &Path, inode: u64, kind: FileKind) -> Result<Self> {
        let ts = SystemTime::now();
        let size = match kind {
            FileKind::File => fs::metadata(path)?.len(),
            FileKind::Directory => 0,
            FileKind::Symlink => unimplemented!(),
        };
        let attr = InodeAttributes {
            inode,
            open_file_handles: 0,
            size,
            last_accessed: ts,
            last_modified: ts,
            last_metadata_changed: ts,
            kind: kind.into(),
            mode: 0o444, // read-only TODO: change once it's read/write
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

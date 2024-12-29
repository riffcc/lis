use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::fuse::{FileKind, InodeAttributes};
use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    /// Absolute path to object
    pub full_path: PathBuf,
    //TODO: pub parent: Inode, (actually put this in attr)
    pub attrs: InodeAttributes,
}

impl Object {
    pub fn new(
        full_path: &Path,
        inode: u64,
        kind: FileKind,
        size: Option<u64>,
        mode: Option<u16>,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<Self> {
        let attrs = match kind {
            FileKind::File => InodeAttributes {
                inode,
                open_file_handles: 0,
                size: size.unwrap_or(0),
                last_accessed: SystemTime::now(),
                last_modified: SystemTime::now(),
                last_metadata_changed: SystemTime::now(),
                kind: kind.into(),
                mode: mode.unwrap_or(0o744),
                hardlinks: 1,
                uid: uid.unwrap_or(unsafe { libc::getuid() }),
                gid: gid.unwrap_or(unsafe { libc::getgid() }),
                xattrs: Default::default(),
            },
            FileKind::Directory => InodeAttributes {
                inode,
                open_file_handles: 0,
                size: BLOCK_SIZE,
                last_accessed: SystemTime::now(),
                last_modified: SystemTime::now(),
                last_metadata_changed: SystemTime::now(),
                kind: kind.into(),
                mode: mode.unwrap_or(0o755),
                hardlinks: 2, // Directories start with link count of 2, since they have a self link
                uid: uid.unwrap_or_else(|| unsafe { libc::getuid() }),
                gid: gid.unwrap_or_else(|| unsafe { libc::getgid() }),
                xattrs: Default::default(),
            },
            FileKind::Symlink => unimplemented!(),
        };
        Ok(Object {
            full_path: full_path.to_path_buf(),
            attrs,
        })
    }
}

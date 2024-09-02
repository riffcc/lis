#[allow(unused)]
use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::prelude::*;

const BLOCK_SIZE: u64 = 512;

impl fuser::Filesystem for Lis {
    fn lookup(
        &mut self,
        _req: &Request<'_>,
        parent: Inode,
        name: &OsStr,
        reply: fuser::ReplyEntry,
    ) {
        debug!("lookup(parent={parent}, name={:#?})", name);
        let full_name = self
            .get_full_name(parent, name)
            .expect("could not get full file name");
        debug!("full_name={}", full_name.display());
        debug!("inodes={:#?}", self.manifest.inodes);

        if let Some(inode) = self.manifest.inodes.get(&full_name) {
            if let Some(obj) = self.manifest.objects.get(inode) {
                let ttl = Duration::new(1, 0);
                reply.entry(&ttl, &obj.attr.clone().into(), 0);
                return;
            }
        }
        reply.error(ENOENT);
    }
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr(ino={ino})");
        let ttl = Duration::new(1, 0);
        if let Some(obj) = self.manifest.objects.get(&ino) {
            reply.attr(&ttl, &obj.attr.clone().into());
        } else {
            reply.error(ENOSYS);
        }
    }
    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir(ino={}, fh={}, offset={})", ino, fh, offset);
        assert!(offset >= 0);

        if let Some(_obj) = self.manifest.objects.get(&ino) {
            if offset == 0 {
                let _ = reply.add(1, 0, FileType::Directory, &Path::new("."));
                let _ = reply.add(1, 1, FileType::Directory, &Path::new(".."));
                let entries = futures::executor::block_on(self.list()).expect("could not list dir");
                for (index, entry) in entries.into_iter().enumerate() {
                    if let Ok(entry) = entry {
                        let key = std::str::from_utf8(entry.key())
                            .expect("Could not go from key to utf8")
                            .replace("\0", "");
                        let path = PathBuf::from(key);
                        let filename = path.file_name().expect("Could not get filename");
                        let _ = reply.add(
                            index as u64 + 2,
                            offset + index as i64 + 1,
                            FileType::RegularFile,
                            &Path::new(filename),
                        );
                    }
                }
            }
            reply.ok();
        } else {
            reply.error(ENOSYS);
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        inode: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!(
            "read() called on {:?} offset={:?} size={:?}",
            inode, offset, size
        );
        assert!(offset >= 0);

        // TODO: check read access
        // if !self.check_file_handle_read(fh) {
        //     reply.error(libc::EACCES);
        //     return;
        // }

        let path = match self.manifest.objects.get(&inode) {
            Some(obj) => obj.path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if let Ok(bytes_content) = futures::executor::block_on(self.get_file(&path)) {
            let buffer: Vec<u8> = bytes_content.to_vec();
            reply.data(&buffer);
            return;
        }
        reply.error(libc::ENOENT);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}
impl From<FileKind> for FileType {
    fn from(kind: FileKind) -> Self {
        match kind {
            FileKind::File => fuser::FileType::RegularFile,
            FileKind::Directory => fuser::FileType::Directory,
            FileKind::Symlink => fuser::FileType::Symlink,
        }
    }
}

pub fn path_kind(path: &Path) -> Result<FileKind> {
    if path.is_file() {
        Ok(FileKind::File)
    } else if path.is_dir() {
        Ok(FileKind::Directory)
    } else if path.is_symlink() {
        Ok(FileKind::Symlink)
    } else {
        Err(anyhow!("unsupported path type"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InodeAttributes {
    pub inode: Inode,
    pub open_file_handles: u64, // Ref count of open file handles to this inode
    pub size: u64,
    pub last_accessed: SystemTime,
    pub last_modified: SystemTime,
    pub last_metadata_changed: SystemTime,
    pub kind: FileKind,
    // Permissions and special mode bits
    pub mode: u16,
    pub hardlinks: u32,
    pub uid: u32,
    pub gid: u32,
    pub xattrs: BTreeMap<Vec<u8>, Vec<u8>>,
}

impl From<InodeAttributes> for fuser::FileAttr {
    fn from(attrs: InodeAttributes) -> Self {
        fuser::FileAttr {
            ino: attrs.inode,
            size: attrs.size,
            blocks: (attrs.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
            atime: attrs.last_accessed,
            mtime: attrs.last_modified,
            ctime: attrs.last_metadata_changed,
            crtime: SystemTime::UNIX_EPOCH,
            kind: attrs.kind.into(),
            perm: attrs.mode,
            nlink: attrs.hardlinks,
            uid: attrs.uid,
            gid: attrs.gid,
            rdev: 0,
            blksize: BLOCK_SIZE as u32,
            flags: 0,
        }
    }
}

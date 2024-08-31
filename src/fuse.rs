// use fuser::{FileAttr, FileType, Filesystem, ReplyAttr, ReplyDirectory, Request, FUSE_ROOT_ID};
use fuser::{FileAttr, FileType, ReplyAttr, ReplyDirectory, Request};
use libc::ENOSYS;
#[allow(unused)]
use log::{debug, error, info, warn, LevelFilter};
#[allow(unused)]
use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::Lis;

impl fuser::Filesystem for Lis {
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr(ino={ino})");
        let ts = SystemTime::now();
        let attr = FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: ts,
            mtime: ts,
            ctime: ts,
            crtime: ts,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 0,
            uid: 0,
            blksize: 512,
            gid: 0,
            rdev: 0,
            flags: 0,
        };
        let ttl = Duration::new(1, 0);
        if ino == 1 {
            reply.attr(&ttl, &attr);
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
        if ino == 1 {
            if offset == 0 {
                let _ = reply.add(1, 0, FileType::Directory, &Path::new("."));
                let _ = reply.add(1, 1, FileType::Directory, &Path::new(".."));
                let handle = tokio::runtime::Handle::current();
                let _guard = handle.enter();
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
}

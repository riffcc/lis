#[allow(unused)]
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::{ffi::OsStrExt, io::IntoRawFd},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use crate::{
    prelude::*,
    util::{key_from_file, key_to_string},
};

impl fuser::Filesystem for Lis {
    fn lookup(&mut self, req: &Request<'_>, parent: Inode, name: &OsStr, reply: fuser::ReplyEntry) {
        debug!("lookup(parent={parent}, name={:#?})", name);
        if name.len() > MAX_NAME_LENGTH as usize {
            reply.error(libc::ENAMETOOLONG);
            return;
        }

        let parent_attr = match self.manifest.objects.get(&parent) {
            Some(obj) => obj.attr.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if !check_access(
            parent_attr.uid,
            parent_attr.gid,
            parent_attr.mode,
            req.uid(),
            req.gid(),
            libc::X_OK,
        ) {
            reply.error(libc::EACCES);
            return;
        }

        let full_path = self
            .get_full_path(parent, name)
            .expect("could not get full file name");

        match self.obj_from_path(&full_path) {
            Some(obj) => reply.entry(&Duration::new(0, 0), &obj.attr.clone().into(), 0),
            None => reply.error(ENOENT),
        }
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
    fn open(&mut self, req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open() called for {:?}", ino);
        let (access_mask, read, write) = match flags & libc::O_ACCMODE {
            libc::O_RDONLY => {
                // Behavior is undefined, but most filesystems return EACCES
                if flags & libc::O_TRUNC != 0 {
                    reply.error(libc::EACCES);
                    return;
                }
                if flags & FMODE_EXEC != 0 {
                    // Open is from internal exec syscall
                    (libc::X_OK, true, false)
                } else {
                    (libc::R_OK, true, false)
                }
            }
            libc::O_WRONLY => (libc::W_OK, false, true),
            libc::O_RDWR => (libc::R_OK | libc::W_OK, true, true),
            // Exactly one access mode flag must be specified
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        match self.manifest.objects.get(&ino) {
            Some(obj) => {
                let mut attr = obj.attr.clone();
                if check_access(
                    attr.uid,
                    attr.gid,
                    attr.mode,
                    req.uid(),
                    req.gid(),
                    access_mask,
                ) {
                    attr.open_file_handles += 1;
                    if let Err(e) = self.write_inode(&attr) {
                        error!("{e}");
                        reply.error(libc::ENOENT);
                        return;
                    }
                    let open_flags = fuser::consts::FOPEN_DIRECT_IO;
                    reply.opened(self.next_file_handle(read, write), open_flags);
                } else {
                    reply.error(libc::EACCES);
                }
                return;
            }
            None => reply.error(libc::ENOENT),
        }
    }

    // fn create(
    //     &mut self,
    //     req: &Request,
    //     parent: u64,
    //     name: &OsStr,
    //     mut mode: u32,
    //     _umask: u32,
    //     flags: i32,
    //     reply: ReplyCreate,
    // ) {
    //     debug!("create() called with {:?} {:?}", parent, name);
    //     let (mut parent_attr, parent_path) = match self.manifest.objects.get(&parent) {
    //         Some(obj) => (obj.attr.clone(), obj.full_path.clone()),
    //         None => {
    //             error!("Could not find parent at inode {parent}");
    //             reply.error(libc::ENOENT);
    //             return;
    //         }
    //     };
    //     let full_path = parent_path.join(name);

    //     // check if file already exists
    //     match handle.block_on(self.list(&parent_path)) {
    //         Ok(entries) => {
    //             let new_path_key = key_from_file(Path::new(""), Path::new(name)).unwrap();
    //             let already_present = entries.iter().any(|entry| match entry {
    //                 Ok(entry) => entry.key() == new_path_key,
    //                 Err(_) => false,
    //             });
    //             if already_present == true {
    //                 reply.error(libc::EEXIST);
    //                 return;
    //             }
    //         }
    //         Err(_e) => {
    //             error!("Could not list entries for {}", parent_path.display());
    //             reply.error(libc::ENOENT);
    //             return;
    //         }
    //     }

    //     let (read, write) = match flags & libc::O_ACCMODE {
    //         libc::O_RDONLY => (true, false),
    //         libc::O_WRONLY => (false, true),
    //         libc::O_RDWR => (true, true),
    //         // Exactly one access mode flag must be specified
    //         _ => {
    //             reply.error(libc::EINVAL);
    //             return;
    //         }
    //     };

    //     if !check_access(
    //         parent_attr.uid,
    //         parent_attr.gid,
    //         parent_attr.mode,
    //         req.uid(),
    //         req.gid(),
    //         libc::W_OK,
    //     ) {
    //         reply.error(libc::EACCES);
    //         return;
    //     }
    //     parent_attr.last_modified = SystemTime::now();
    //     parent_attr.last_metadata_changed = SystemTime::now();
    //     self.write_inode(&parent_attr);

    //     if req.uid() != 0 {
    //         mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
    //     }

    //     let inode = self.next_ino();
    //     let attr = InodeAttributes {
    //         inode,
    //         open_file_handles: 1,
    //         size: 0,
    //         last_accessed: SystemTime::now(),
    //         last_modified: SystemTime::now(),
    //         last_metadata_changed: SystemTime::now(),
    //         kind: as_file_kind(mode),
    //         mode: self.creation_mode(mode),
    //         hardlinks: 1,
    //         uid: req.uid(),
    //         gid: creation_gid(&parent_attr, req.gid()),
    //         xattrs: Default::default(),
    //     };
    //     self.write_inode(&attr);
    //     File::create(self.content_path(inode)).unwrap();

    //     // TODO: put file to lis

    //     reply.created(
    //         &Duration::new(0, 0),
    //         &attr.into(),
    //         0,
    //         self.next_file_handle(read, write),
    //         0,
    //     );
    // }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
    }

    fn fallocate(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        length: i64,
        mode: i32,
        reply: fuser::ReplyEmpty,
    ) {
        let (path, mut attr) = match self.manifest.objects.get(&ino) {
            Some(obj) => (obj.full_path.to_path_buf(), obj.attr.clone()),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if let Ok(file) = OpenOptions::new().write(true).open(path) {
            unsafe {
                libc::fallocate64(file.into_raw_fd(), mode, offset, length);
            }
            if mode & libc::FALLOC_FL_KEEP_SIZE == 0 {
                attr.last_metadata_changed = SystemTime::now();
                attr.last_modified = SystemTime::now();
                if (offset + length) as u64 > attr.size {
                    attr.size = (offset + length) as u64;
                }
                if let Err(e) = self.write_inode(&attr) {
                    error!("{e}");
                    reply.error(libc::ENOENT);
                    return;
                }
            }
            reply.ok();
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn opendir(&mut self, req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("opendir() called on {:?}", ino);
        let (access_mask, read, write) = match flags & libc::O_ACCMODE {
            libc::O_RDONLY => {
                // Behavior is undefined, but most filesystems return EACCES
                if flags & libc::O_TRUNC != 0 {
                    reply.error(libc::EACCES);
                    return;
                }
                (libc::R_OK, true, false)
            }
            libc::O_WRONLY => (libc::W_OK, false, true),
            libc::O_RDWR => (libc::R_OK | libc::W_OK, true, true),
            // Exactly one access mode flag must be specified
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        match self.manifest.objects.get(&ino) {
            Some(obj) => {
                let mut attr = obj.attr.clone();
                if check_access(
                    attr.uid,
                    attr.gid,
                    attr.mode,
                    req.uid(),
                    req.gid(),
                    access_mask,
                ) {
                    attr.open_file_handles += 1;
                    if let Err(e) = self.write_inode(&attr) {
                        error!("{e}");
                        reply.error(libc::ENOENT);
                        return;
                    }
                    let open_flags = fuser::consts::FOPEN_DIRECT_IO;
                    reply.opened(self.next_file_handle(read, write), open_flags);
                } else {
                    reply.error(libc::EACCES);
                }
                return;
            }
            None => reply.error(libc::ENOENT),
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

        let handle = self.rt.clone();

        if offset != 0 {
            reply.ok();
            return;
        }

        if let Some(obj) = self.manifest.objects.get(&ino) {
            let _ = reply.add(ino, 0, FileType::Directory, &Path::new("."));
            let _ = reply.add(ino, 1, FileType::Directory, &Path::new(".."));
            let entries = handle
                .block_on(self.list(&obj.full_path))
                .expect("could not list dir");

            for (index, entry) in entries.into_iter().enumerate() {
                if let Ok(entry) = entry {
                    let skey = key_to_string(entry.key().to_vec().into())
                        .expect("Could not go from key to string");
                    let relpath = PathBuf::from(skey);
                    let full_entry_path = obj.full_path.join(relpath.clone());

                    match self.obj_from_path(&full_entry_path) {
                        Some(obj) => {
                            let _ = reply.add(
                                obj.attr.inode,
                                offset + index as i64 + 1,
                                obj.attr.kind.into(),
                                &relpath.clone(),
                            );
                        }
                        None => {
                            let entry_ino = self
                                .manifest
                                .inodes
                                .get(&full_entry_path.to_path_buf())
                                .unwrap_or(&0);
                            error!(
                                "Cannot find object from path {} (ino={})",
                                full_entry_path.display(),
                                entry_ino
                            );
                            debug!("{:#?}", self.manifest);
                            reply.error(ENOSYS);
                            return;
                        }
                    }
                }
            }
            reply.ok();
        } else {
            error!("Cannot find object at inode {ino}");
            reply.error(ENOSYS);
        }
    }

    fn mkdir(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mut mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        debug!("mkdir() called with {:?} {:?} {:o}", parent, name, mode);
        let handle = self.rt.clone();

        // get object of parent
        // get path of parent from object
        let (mut parent_attr, parent_path) = match self.manifest.objects.get(&parent) {
            Some(obj) => (obj.attr.clone(), obj.full_path.clone()),
            None => {
                error!("Could not find parent at inode {parent}");
                reply.error(libc::ENOENT);
                return;
            }
        };
        let full_path = parent_path.join(name);

        // check if can access parent dir
        if !check_access(
            parent_attr.uid,
            parent_attr.gid,
            parent_attr.mode,
            req.uid(),
            req.gid(),
            libc::W_OK,
        ) {
            error!("Access denied for {}", parent_path.display());
            reply.error(libc::EACCES);
            return;
        }

        // check if directory already exists
        match handle.block_on(self.list(&parent_path)) {
            Ok(entries) => {
                let new_path_key = key_from_file(Path::new(""), Path::new(name)).unwrap();
                let already_present = entries.iter().any(|entry| match entry {
                    Ok(entry) => entry.key() == new_path_key,
                    Err(_) => false,
                });
                if already_present == true {
                    reply.error(libc::EEXIST);
                    return;
                }
            }
            Err(_e) => {
                error!("Could not list entries for {}", parent_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        }

        // create dir
        if req.uid() != 0 {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }
        if parent_attr.mode & libc::S_ISGID as u16 != 0 {
            mode |= libc::S_ISGID as u32;
        }
        let uid = req.uid();
        let gid = creation_gid(&parent_attr, req.gid());
        if let Err(e) =
            handle.block_on(self.mkdir(&full_path, Some(mode as u16), Some(uid), Some(gid)))
        {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }

        // update parent attributes
        parent_attr.last_modified = SystemTime::now();
        parent_attr.last_metadata_changed = SystemTime::now();
        if let Err(e) = self.write_inode(&parent_attr) {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }

        // return attrs of newly created dir
        let attr = match self.obj_from_path(&full_path) {
            Some(obj) => obj.attr.clone(),
            None => {
                error!("Could not find newly created dir {}", full_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        };

        reply.entry(&Duration::new(0, 0), &attr.into(), 0);
    }

    fn read(
        &mut self,
        _req: &Request,
        inode: u64,
        fh: u64,
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
        let handle = self.rt.clone();

        if !check_file_handle_read(fh) {
            reply.error(libc::EACCES);
            return;
        }

        let path = match self.manifest.objects.get(&inode) {
            Some(obj) => obj.full_path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if let Ok(bytes_content) = handle.block_on(self.get_file(&path)) {
            let buffer: Vec<u8> = bytes_content.to_vec();
            reply.data(&buffer);
            return;
        }
        reply.error(libc::ENOENT);
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        debug!("write() called with {:?} size={:?}", ino, data.len());
        assert!(offset >= 0);
        if !check_file_handle_write(fh) {
            reply.error(libc::EACCES);
            return;
        }

        let (obj, path) = match self.manifest.objects.get(&ino) {
            Some(obj) => (obj, obj.full_path.clone()),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        if let Ok(mut file) = OpenOptions::new().write(true).open(path) {
            file.seek(SeekFrom::Start(offset as u64)).unwrap();
            file.write_all(data).unwrap();

            let mut attr = obj.attr.clone();
            attr.last_metadata_changed = SystemTime::now();
            attr.last_modified = SystemTime::now();
            if data.len() + offset as usize > attr.size as usize {
                attr.size = (data.len() + offset as usize) as u64;
            }
            clear_suid_sgid(&mut attr);
            if let Err(e) = self.write_inode(&attr) {
                error!("Could not write: {e}");
                reply.error(libc::ENOENT);
                return;
            }

            reply.written(data.len() as u32);
        } else {
            reply.error(libc::EBADF);
        }
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

#[allow(unused)]
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

fn creation_gid(parent: &InodeAttributes, gid: u32) -> u32 {
    if parent.mode & libc::S_ISGID as u16 != 0 {
        return parent.gid;
    }

    gid
}

fn clear_suid_sgid(attr: &mut InodeAttributes) {
    attr.mode &= !libc::S_ISUID as u16;
    // SGID is only suppose to be cleared if XGRP is set
    if attr.mode & libc::S_IXGRP as u16 != 0 {
        attr.mode &= !libc::S_ISGID as u16;
    }
}

pub fn check_access(
    file_uid: u32,
    file_gid: u32,
    file_mode: u16,
    uid: u32,
    gid: u32,
    mut access_mask: i32,
) -> bool {
    // F_OK tests for existence of file
    if access_mask == libc::F_OK {
        return true;
    }
    let file_mode = i32::from(file_mode);

    // root is allowed to read & write anything
    if uid == 0 {
        // root only allowed to exec if one of the X bits is set
        access_mask &= libc::X_OK;
        access_mask -= access_mask & (file_mode >> 6);
        access_mask -= access_mask & (file_mode >> 3);
        access_mask -= access_mask & file_mode;
        return access_mask == 0;
    }

    if uid == file_uid {
        access_mask -= access_mask & (file_mode >> 6);
    } else if gid == file_gid {
        access_mask -= access_mask & (file_mode >> 3);
    } else {
        access_mask -= access_mask & file_mode;
    }

    return access_mask == 0;
}

fn check_file_handle_read(file_handle: u64) -> bool {
    (file_handle & FILE_HANDLE_READ_BIT) != 0
}
fn check_file_handle_write(file_handle: u64) -> bool {
    (file_handle & FILE_HANDLE_WRITE_BIT) != 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InodeAttributes {
    // TODO: pub parent: Inode,
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

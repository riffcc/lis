#[allow(unused)]
use std::{
    cmp::min,
    ffi::OsStr,
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::FileExt, io::IntoRawFd},
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use fuser::TimeOrNow;
use fuser::TimeOrNow::Now;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
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

        let parent_attrs = match self.manifest.objects.get(&parent) {
            Some(obj) => obj.attrs.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        if !check_access(
            parent_attrs.uid,
            parent_attrs.gid,
            parent_attrs.mode,
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
            Some(obj) => reply.entry(&Duration::new(0, 0), &obj.attrs.clone().into(), 0),
            None => reply.error(ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr(ino={ino})");
        match self.manifest.objects.get(&ino) {
            Some(obj) => reply.attr(&Duration::new(1, 0), &obj.attrs.clone().into()),
            None => reply.error(ENOSYS),
        }
    }

    fn open(&mut self, req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        debug!("open(ino={ino})");
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
                let mut attrs = obj.attrs.clone();
                if check_access(
                    attrs.uid,
                    attrs.gid,
                    attrs.mode,
                    req.uid(),
                    req.gid(),
                    access_mask,
                ) {
                    attrs.open_file_handles += 1;
                    if let Err(e) = self.write_inode(&attrs) {
                        error!("{e}");
                        reply.error(libc::ENOENT);
                        return;
                    }
                    let open_flags = fuser::consts::FOPEN_DIRECT_IO;
                    reply.opened(self.next_file_handle(read, write), open_flags);
                } else {
                    reply.error(libc::EACCES);
                }
            }
            None => reply.error(libc::ENOENT),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        if let Some(obj) = self.manifest.objects.get(&ino) {
            let mut attrs = obj.attrs.clone();
            attrs.open_file_handles -= 1;
            if let Err(e) = self.write_inode(&attrs) {
                error!("{e}");
                reply.error(libc::ENOENT);
                return;
            }
        }
        reply.ok();
    }

    fn create(
        &mut self,
        req: &Request,
        parent: u64,
        name: &OsStr,
        mut mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        debug!("create(parent={:?}, name={:?})", parent, name);

        let handle = self.rt.clone();

        let (mut parent_attrs, parent_path) = match self.manifest.objects.get(&parent) {
            Some(obj) => (obj.attrs.clone(), obj.full_path.clone()),
            None => {
                error!("Could not find parent at inode {parent}");
                reply.error(libc::ENOENT);
                return;
            }
        };

        // check if file already exists
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

        let (read, write) = match flags & libc::O_ACCMODE {
            libc::O_RDONLY => (true, false),
            libc::O_WRONLY => (false, true),
            libc::O_RDWR => (true, true),
            // Exactly one access mode flag must be specified
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let full_path = parent_path.join(name);
        if !check_access(
            parent_attrs.uid,
            parent_attrs.gid,
            parent_attrs.mode,
            req.uid(),
            req.gid(),
            libc::W_OK,
        ) {
            reply.error(libc::EACCES);
            return;
        }

        let uid = req.uid();
        let gid = creation_gid(&parent_attrs, req.gid());

        if uid != 0 {
            mode &= !(libc::S_ISUID | libc::S_ISGID) as u32;
        }

        // Create new file on lis
        if let Err(e) =
            handle.block_on(self.touch(&full_path, Some(mode as u16), Some(uid), Some(gid)))
        {
            error!("Could not put file on lis: {e}");
            reply.error(libc::ENOENT);
            return;
        }

        parent_attrs.last_modified = SystemTime::now();
        parent_attrs.last_metadata_changed = SystemTime::now();
        if let Err(e) = self.write_inode(&parent_attrs) {
            error!("Could not write inode: {e}");
            reply.error(libc::ENOENT);
            return;
        }

        // get attrs of newly created file
        let mut attrs = match self.obj_from_path(&full_path) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find newly created dir {}", full_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        };
        // update attrs of file
        attrs.open_file_handles = 1;
        if let Err(e) = self.write_inode(&attrs) {
            error!("Could not write inode: {e}");
            reply.error(libc::ENOENT);
            return;
        }

        reply.created(
            &Duration::new(0, 0),
            &attrs.into(),
            0,
            self.next_file_handle(read, write),
            0,
        );
    }

    fn unlink(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("unlink(parent={parent}, name={:#?}", name);

        let handle = self.rt.clone();

        let mut parent_attrs = match self.manifest.objects.get(&parent) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find inode {parent}");
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = self
            .get_full_path(parent, name)
            .expect("could not get full file name");

        let mut attrs = match self.obj_from_path(&full_path) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find newly created dir {}", full_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        };

        if !check_access(
            parent_attrs.uid,
            parent_attrs.gid,
            parent_attrs.mode,
            req.uid(),
            req.gid(),
            libc::W_OK,
        ) {
            reply.error(libc::EACCES);
            return;
        }

        let uid = req.uid();
        // "Sticky bit" handling
        if parent_attrs.mode & libc::S_ISVTX as u16 != 0
            && uid != 0
            && uid != parent_attrs.uid
            && uid != attrs.uid
        {
            reply.error(libc::EACCES);
            return;
        }

        if handle.block_on(self.remove(&full_path)).is_err() {
            error!("Could not remove from lis");
            reply.error(libc::ENOENT);
            return;
        }

        parent_attrs.last_metadata_changed = SystemTime::now();
        parent_attrs.last_modified = SystemTime::now();
        if let Err(e) = self.write_inode(&parent_attrs) {
            error!("{e}");
            reply.error(libc::ENOENT);
            return;
        }

        attrs.hardlinks -= 1;
        attrs.last_metadata_changed = SystemTime::now();
        if let Err(e) = self.write_inode(&attrs) {
            error!("{e}");
            reply.error(libc::ENOENT);
            return;
        }

        if let Err(e) = self.gc_inode(&attrs) {
            error!("{e}");
            reply.error(libc::ENOENT);
            return;
        }

        reply.ok();
    }

    fn setattr(
        &mut self,
        req: &Request,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let handle = self.rt.clone();

        debug!("setattr(ino={ino})");
        let mut attrs = match self.manifest.objects.get(&ino) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find inode {ino}");
                reply.error(libc::ENOENT);
                return;
            }
        };

        if let Some(mode) = mode {
            debug!("chmod() called with {:?}, {:o}", ino, mode);
            if req.uid() != 0 && req.uid() != attrs.uid {
                reply.error(libc::EPERM);
                return;
            }
            if req.uid() != 0
                && req.gid() != attrs.gid
                && !handle.block_on(get_groups(req.pid())).contains(&attrs.gid)
            {
                // If SGID is set and the file belongs to a group that the caller is not part of
                // then the SGID bit is suppose to be cleared during chmod
                attrs.mode = (mode & !libc::S_ISGID as u32) as u16;
            } else {
                attrs.mode = mode as u16;
            }
            attrs.last_metadata_changed = SystemTime::now();
            if let Err(e) = self.write_inode(&attrs) {
                error!("{e}");
                reply.error(libc::ENOENT);
                return;
            }
            reply.attr(&Duration::new(0, 0), &attrs.into());
            return;
        }

        if uid.is_some() || gid.is_some() {
            debug!("chown() called with {:?} {:?} {:?}", ino, uid, gid);
            if let Some(gid) = gid {
                // Non-root users can only change gid to a group they're in
                if req.uid() != 0 && !handle.block_on(get_groups(req.pid())).contains(&gid) {
                    reply.error(libc::EPERM);
                    return;
                }
            }
            if let Some(uid) = uid {
                if req.uid() != 0
                    // but no-op changes by the owner are not an error
                    && !(uid == attrs.uid && req.uid() == attrs.uid)
                {
                    reply.error(libc::EPERM);
                    return;
                }
            }
            // Only owner may change the group
            if gid.is_some() && req.uid() != 0 && req.uid() != attrs.uid {
                reply.error(libc::EPERM);
                return;
            }

            if attrs.mode & (libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH) as u16 != 0 {
                // SUID & SGID are suppose to be cleared when chown'ing an executable file
                clear_suid_sgid(&mut attrs);
            }

            if let Some(uid) = uid {
                attrs.uid = uid;
                // Clear SETUID on owner change
                attrs.mode &= !libc::S_ISUID as u16;
            }
            if let Some(gid) = gid {
                attrs.gid = gid;
                // Clear SETGID unless user is root
                if req.uid() != 0 {
                    attrs.mode &= !libc::S_ISGID as u16;
                }
            }
            attrs.last_metadata_changed = SystemTime::now();
            if let Err(e) = self.write_inode(&attrs) {
                error!("{e}");
                reply.error(libc::ENOENT);
                return;
            }
            reply.attr(&Duration::new(0, 0), &attrs.into());
            return;
        }

        if let Some(size) = size {
            debug!("truncate(ino={ino}, size={size})");
            if let Some(file_handle) = fh {
                // If the file handle is available, check access locally.
                // This is important as it preserves the semantic that a file handle opened
                // with W_OK will never fail to truncate, even if the file has been subsequently
                // chmod'ed
                if check_file_handle_write(file_handle) {
                    if let Err(error_code) = handle.block_on(self.truncate(ino, size, 0, 0)) {
                        reply.error(error_code);
                        return;
                    }
                } else {
                    reply.error(libc::EACCES);
                    return;
                }
            } else if let Err(error_code) =
                handle.block_on(self.truncate(ino, size, req.uid(), req.gid()))
            {
                reply.error(error_code);
                return;
            }
        }

        let now = SystemTime::now();
        if let Some(atime) = atime {
            debug!("utimens() called with {:?}, atime={:?}", ino, atime);

            if attrs.uid != req.uid() && req.uid() != 0 && atime != Now {
                reply.error(libc::EPERM);
                return;
            }

            if attrs.uid != req.uid()
                && !check_access(
                    attrs.uid,
                    attrs.gid,
                    attrs.mode,
                    req.uid(),
                    req.gid(),
                    libc::W_OK,
                )
            {
                reply.error(libc::EACCES);
                return;
            }

            attrs.last_accessed = match atime {
                TimeOrNow::SpecificTime(time) => time,
                Now => now,
            };
            attrs.last_metadata_changed = SystemTime::now();
            if let Err(e) = self.write_inode(&attrs) {
                error!("{e}");
                reply.error(libc::ENOENT);
                return;
            }
        }
        if let Some(mtime) = mtime {
            debug!("utimens() called with {:?}, mtime={:?}", ino, mtime);

            if attrs.uid != req.uid() && req.uid() != 0 && mtime != Now {
                reply.error(libc::EPERM);
                return;
            }

            if attrs.uid != req.uid()
                && !check_access(
                    attrs.uid,
                    attrs.gid,
                    attrs.mode,
                    req.uid(),
                    req.gid(),
                    libc::W_OK,
                )
            {
                reply.error(libc::EACCES);
                return;
            }

            attrs.last_modified = match mtime {
                TimeOrNow::SpecificTime(time) => time,
                Now => now,
            };
            attrs.last_metadata_changed = now;
            if let Err(e) = self.write_inode(&attrs) {
                error!("{e}");
                reply.error(libc::ENOENT);
                return;
            }
        }

        // save new attributes
        match self.write_inode(&attrs) {
            Ok(_) => reply.attr(&Duration::new(0, 0), &attrs.into()),
            Err(e) => {
                error!("{e}");
                reply.error(libc::ENOENT);
            }
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
                let mut attrs = obj.attrs.clone();
                if check_access(
                    attrs.uid,
                    attrs.gid,
                    attrs.mode,
                    req.uid(),
                    req.gid(),
                    access_mask,
                ) {
                    attrs.open_file_handles += 1;
                    if let Err(e) = self.write_inode(&attrs) {
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
                                obj.attrs.inode,
                                offset + index as i64 + 1,
                                obj.attrs.kind.into(),
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

    fn rmdir(&mut self, req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        debug!("rmdir(parent={parent}, name={:#?}", name);

        let handle = self.rt.clone();

        let full_path = self
            .get_full_path(parent, name)
            .expect("could not get full file name");

        let mut attrs = match self.obj_from_path(&full_path) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find newly created dir {}", full_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        };

        let mut parent_attrs = match self.manifest.objects.get(&parent) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find inode {parent}");
                reply.error(libc::ENOENT);
                return;
            }
        };

        // lis rmdir
        // TODO: other error type for when trying to rm root dir
        if let Err(e) = handle.block_on(self.rmdir(&full_path)) {
            error!("Unable to rmdir {}: {e}", full_path.display());
            reply.error(libc::ENOTEMPTY);
            return;
        }

        if !check_access(
            parent_attrs.uid,
            parent_attrs.gid,
            parent_attrs.mode,
            req.uid(),
            req.gid(),
            libc::W_OK,
        ) {
            reply.error(libc::EACCES);
            return;
        }

        // "Sticky bit" handling
        if parent_attrs.mode & libc::S_ISVTX as u16 != 0
            && req.uid() != 0
            && req.uid() != parent_attrs.uid
            && req.uid() != attrs.uid
        {
            reply.error(libc::EACCES);
            return;
        }

        parent_attrs.last_metadata_changed = SystemTime::now();
        parent_attrs.last_modified = SystemTime::now();
        if let Err(e) = self.write_inode(&parent_attrs) {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }

        attrs.hardlinks = 0;
        attrs.last_metadata_changed = SystemTime::now();
        if let Err(e) = self.write_inode(&attrs) {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }
        if let Err(e) = self.gc_inode(&attrs) {
            error!("{e}");
            reply.error(libc::ENOENT);
            return;
        }

        reply.ok();
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
        debug!("mkdir(parent={parent}, name={:#?}, mode={:o})", name, mode);
        let handle = self.rt.clone();

        let (mut parent_attrs, parent_path) = match self.manifest.objects.get(&parent) {
            Some(obj) => (obj.attrs.clone(), obj.full_path.clone()),
            None => {
                error!("Could not find parent at inode {parent}");
                reply.error(libc::ENOENT);
                return;
            }
        };
        let full_path = parent_path.join(name);

        // check if can access parent dir
        if !check_access(
            parent_attrs.uid,
            parent_attrs.gid,
            parent_attrs.mode,
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
        if parent_attrs.mode & libc::S_ISGID as u16 != 0 {
            mode |= libc::S_ISGID as u32;
        }
        let uid = req.uid();
        let gid = creation_gid(&parent_attrs, req.gid());
        if let Err(e) =
            handle.block_on(self.mkdir(&full_path, Some(mode as u16), Some(uid), Some(gid)))
        {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }

        // update parent attributes
        parent_attrs.last_modified = SystemTime::now();
        parent_attrs.last_metadata_changed = SystemTime::now();
        if let Err(e) = self.write_inode(&parent_attrs) {
            error!("Could not create dir {}: {e}", full_path.display());
            reply.error(libc::ENOENT);
            return;
        }

        // return attrs of newly created dir
        let attrs = match self.obj_from_path(&full_path) {
            Some(obj) => obj.attrs.clone(),
            None => {
                error!("Could not find newly created dir {}", full_path.display());
                reply.error(libc::ENOENT);
                return;
            }
        };

        reply.entry(&Duration::new(0, 0), &attrs.into(), 0);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read(ino={ino} offset={offset} size={size})");
        assert!(offset >= 0);
        let handle = self.rt.clone();

        if !check_file_handle_read(fh) {
            reply.error(libc::EACCES);
            return;
        }

        let path = match self.manifest.objects.get(&ino) {
            Some(obj) => obj.full_path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let offset = offset as usize;
        match handle.block_on(self.read(&path)) {
            Ok(bytes_content) => {
                let content_size = bytes_content.len();
                let read_size: usize =
                    min(size, content_size.saturating_sub(offset) as u32) as usize;
                let buffer = bytes_content.slice(offset..(offset + read_size));
                reply.data(&buffer);
            }
            Err(e) => {
                error!("Could not get file: {e}");
                reply.error(libc::ENOENT);
            }
        }
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
        let handle = self.rt.clone();

        debug!("write(ino={ino}, size={:?})", data.len());
        assert!(offset >= 0);
        if !check_file_handle_write(fh) {
            reply.error(libc::EACCES);
            return;
        }

        let (mut attrs, full_path) = match self.manifest.objects.get(&ino) {
            Some(obj) => (obj.attrs.clone(), obj.full_path.clone()),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // save data to lis
        if handle
            .block_on(self.write(&full_path, data, offset as usize))
            .is_ok()
        {
            // update attributes
            attrs.last_metadata_changed = SystemTime::now();
            attrs.last_modified = SystemTime::now();
            if data.len() + offset as usize > attrs.size as usize {
                attrs.size = (data.len() + offset as usize) as u64;
            }

            clear_suid_sgid(&mut attrs);

            if let Err(e) = self.write_inode(&attrs) {
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

pub fn clear_suid_sgid(attrs: &mut InodeAttributes) {
    attrs.mode &= !libc::S_ISUID as u16;
    // SGID is only suppose to be cleared if XGRP is set
    if attrs.mode & libc::S_IXGRP as u16 != 0 {
        attrs.mode &= !libc::S_ISGID as u16;
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
async fn get_groups(pid: u32) -> Vec<u32> {
    {
        let path = format!("/proc/{pid}/task/{pid}/status");
        let file = File::open(path).await.unwrap();
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.starts_with("Groups:") {
                return line["Groups: ".len()..]
                    .split(' ')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| x.parse::<u32>().unwrap())
                    .collect();
            }
        }
    }

    vec![]
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

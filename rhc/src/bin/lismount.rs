use rhc::{
    crypto::Signature,
    lease::{Domain, Lease, LeaseProof},
    message::{Operation, OperationType},
    node::{NodeRole, RhcNode},
    storage::InMemoryStorage,
    time::{HybridClock, HybridTimestamp},
    NodeId, Result,
};
use fuser::{
    Filesystem, Request, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, ReplyWrite,
    ReplyCreate, FileAttr, FileType, MountOption,
};
use libc::{ENOENT, ENOTDIR};
use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing::{error, info};

const TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct FileEntry {
    ino: u64,
    name: String,
    data: Vec<u8>,
    attr: FileAttr,
}

pub struct LisFilesystem {
    rhc_node: Arc<RhcNode>,
    next_ino: Arc<Mutex<u64>>,
    inodes: Arc<Mutex<HashMap<u64, FileEntry>>>,
    path_to_ino: Arc<Mutex<HashMap<PathBuf, u64>>>,
    rt_handle: tokio::runtime::Handle,
}

impl LisFilesystem {
    pub fn new(rhc_node: Arc<RhcNode>, rt_handle: tokio::runtime::Handle) -> Self {
        // Create root directory
        let root_attr = FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            flags: 0,
            blksize: 512,
        };

        let root_entry = FileEntry {
            ino: 1,
            name: "/".to_string(),
            data: Vec::new(),
            attr: root_attr,
        };

        let mut inodes = HashMap::new();
        let mut path_to_ino = HashMap::new();
        inodes.insert(1, root_entry);
        path_to_ino.insert(PathBuf::from("/"), 1);

        Self {
            rhc_node,
            next_ino: Arc::new(Mutex::new(2)),
            inodes: Arc::new(Mutex::new(inodes)),
            path_to_ino: Arc::new(Mutex::new(path_to_ino)),
            rt_handle,
        }
    }

    fn path_from_ino(&self, ino: u64) -> Option<PathBuf> {
        let path_map = self.path_to_ino.lock().unwrap();
        path_map.iter()
            .find(|&(_, &inode)| inode == ino)
            .map(|(path, _)| path.clone())
    }

    fn get_next_ino(&self) -> u64 {
        let mut next = self.next_ino.lock().unwrap();
        let ino = *next;
        *next += 1;
        ino
    }

    fn rhc_read(&self, path: &Path) -> Option<Vec<u8>> {
        let path_str = path.to_string_lossy().to_string();
        info!("RHC read: {}", path_str);
        
        // Use runtime handle to execute async operations from sync context
        let storage = self.rhc_node.storage();
        match self.rt_handle.block_on(storage.get(&path_str)) {
            Ok(Some(data)) => Some(data),
            Ok(None) => None,
            Err(e) => {
                error!("RHC read error for {}: {}", path_str, e);
                None
            }
        }
    }

    fn rhc_write(&self, path: &Path, data: &[u8]) -> bool {
        let path_str = path.to_string_lossy().to_string();
        info!("RHC write: {} ({} bytes)", path_str, data.len());
        
        // For MVP, create a simple write operation
        // In production, this would use proper lease management
        let storage = self.rhc_node.storage();
        let data_owned = data.to_vec();
        
        // Create a write operation
        let operation_data = bincode::serialize(&(path_str.clone(), data_owned))
            .unwrap_or_else(|_| Vec::new());
            
        let operation = Operation {
            id: uuid::Uuid::new_v4(),
            op_type: OperationType::Write,
            data: operation_data,
            lease_proof: LeaseProof {
                lease: Lease {
                    id: uuid::Uuid::new_v4(),
                    domain: Domain::new("root".to_string(), None, 0),
                    holder: NodeId::new(),
                    start_time: HybridClock::new().now(),
                    duration: chrono::Duration::minutes(10),
                    parent_lease: None,
                    signature: Signature::default(),
                },
                chain: vec![],
            },
            timestamp: HybridClock::new().now(),
        };
        
        match self.rt_handle.block_on(storage.apply_operation(&operation)) {
            Ok(_) => {
                info!("RHC write successful for {}", path_str);
                true
            }
            Err(e) => {
                error!("RHC write error for {}: {}", path_str, e);
                false
            }
        }
    }
}

impl Filesystem for LisFilesystem {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = name.to_string_lossy();
        info!("lookup: parent={}, name={}", parent, name_str);
        
        let parent_path = match self.path_from_ino(parent) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        let full_path = parent_path.join(&*name_str);
        
        // Check if already in cache
        {
            let path_map = self.path_to_ino.lock().unwrap();
            if let Some(&ino) = path_map.get(&full_path) {
                let inodes = self.inodes.lock().unwrap();
                if let Some(entry) = inodes.get(&ino) {
                    reply.entry(&TTL, &entry.attr, 0);
                    return;
                }
            }
        }
        
        // Try to read from RHC
        if let Some(data) = self.rhc_read(&full_path) {
            let ino = self.get_next_ino();
            let attr = FileAttr {
                ino,
                size: data.len() as u64,
                blocks: (data.len() as u64 + 511) / 512,
                atime: SystemTime::now(),
                mtime: SystemTime::now(),
                ctime: SystemTime::now(),
                crtime: SystemTime::now(),
                kind: FileType::RegularFile,
                perm: 0o644,
                nlink: 1,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                flags: 0,
                blksize: 512,
            };
            
            let entry = FileEntry {
                ino,
                name: name_str.to_string(),
                data,
                attr,
            };
            
            // Cache the entry
            {
                let mut inodes = self.inodes.lock().unwrap();
                let mut path_map = self.path_to_ino.lock().unwrap();
                inodes.insert(ino, entry.clone());
                path_map.insert(full_path, ino);
            }
            
            reply.entry(&TTL, &entry.attr, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let inodes = self.inodes.lock().unwrap();
        match inodes.get(&ino) {
            Some(entry) => reply.attr(&TTL, &entry.attr),
            None => reply.error(ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        let inodes = self.inodes.lock().unwrap();
        match inodes.get(&ino) {
            Some(entry) => {
                let offset = offset as usize;
                let size = size as usize;
                if offset < entry.data.len() {
                    let end = std::cmp::min(offset + size, entry.data.len());
                    reply.data(&entry.data[offset..end]);
                } else {
                    reply.data(&[]);
                }
            }
            None => reply.error(ENOENT),
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let path = match self.path_from_ino(ino) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // For simplicity, we'll do a full write (not partial)
        if self.rhc_write(&path, data) {
            // Update cached entry
            {
                let mut inodes = self.inodes.lock().unwrap();
                if let Some(entry) = inodes.get_mut(&ino) {
                    if offset == 0 {
                        entry.data = data.to_vec();
                        entry.attr.size = data.len() as u64;
                        entry.attr.mtime = SystemTime::now();
                    }
                }
            }
            reply.written(data.len() as u32);
        } else {
            reply.error(libc::EIO);
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name_str = name.to_string_lossy();
        info!("create: parent={}, name={}", parent, name_str);
        
        let parent_path = match self.path_from_ino(parent) {
            Some(path) => path,
            None => {
                reply.error(ENOENT);
                return;
            }
        };
        
        let full_path = parent_path.join(&*name_str);
        
        // Create empty file in RHC
        if self.rhc_write(&full_path, &[]) {
            let ino = self.get_next_ino();
            let attr = FileAttr {
                ino,
                size: 0,
                blocks: 0,
                atime: SystemTime::now(),
                mtime: SystemTime::now(),
                ctime: SystemTime::now(),
                crtime: SystemTime::now(),
                kind: FileType::RegularFile,
                perm: 0o644,
                nlink: 1,
                uid: unsafe { libc::getuid() },
                gid: unsafe { libc::getgid() },
                rdev: 0,
                flags: 0,
                blksize: 512,
            };
            
            let entry = FileEntry {
                ino,
                name: name_str.to_string(),
                data: Vec::new(),
                attr,
            };
            
            // Cache the entry
            {
                let mut inodes = self.inodes.lock().unwrap();
                let mut path_map = self.path_to_ino.lock().unwrap();
                inodes.insert(ino, entry.clone());
                path_map.insert(full_path, ino);
            }
            
            reply.created(&TTL, &entry.attr, 0, 0, 0);
        } else {
            reply.error(libc::EIO);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino == 1 {
            // Root directory - list all cached files
            let inodes = self.inodes.lock().unwrap();
            let mut entries: Vec<_> = inodes.values().collect();
            entries.sort_by_key(|e| e.ino);
            
            let mut index = 0;
            for entry in entries {
                if index >= offset {
                    if reply.add(entry.ino, index + 1, entry.attr.kind, &entry.name) {
                        break;
                    }
                }
                index += 1;
            }
            reply.ok();
        } else {
            reply.error(ENOTDIR);
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        let inodes = self.inodes.lock().unwrap();
        match inodes.get(&ino) {
            Some(_) => reply.opened(0, 0),
            None => reply.error(ENOENT),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <cluster> <mountpoint>", args[0]);
        eprintln!("Example: {} lis-cluster:/ /mnt/lis", args[0]);
        std::process::exit(1);
    }
    
    let cluster = &args[1];
    let mountpoint = &args[2];
    
    info!("LIS Mount starting...");
    info!("Cluster: {}", cluster);
    info!("Mountpoint: {}", mountpoint);
    
    // Create RHC node - for MVP, use LocalLeader with auto-discovery
    let node_id = NodeId::new();
    let rhc_node = Arc::new(RhcNode::new(
        NodeRole::LocalLeader,
        1, // Level 1 - Local leader
        Arc::new(InMemoryStorage::new()),
        None, // Will discover peers automatically
    ));
    
    // Start RHC node
    rhc_node.start().await?;
    info!("RHC node {:?} started", node_id);
    
    // Create filesystem
    let rt_handle = tokio::runtime::Handle::current();
    let fs = LisFilesystem::new(rhc_node.clone(), rt_handle);
    
    // Mount options for better performance
    let options = vec![
        MountOption::RW,
        MountOption::FSName("lis".to_string()),
        MountOption::AllowOther,
        MountOption::AutoUnmount,
    ];
    
    info!("Mounting Lis filesystem at {}", mountpoint);
    
    // This will block and run the filesystem
    fuser::mount2(fs, mountpoint, &options)
        .map_err(|e| anyhow::anyhow!("Failed to mount filesystem: {}", e))?;
    
    Ok(())
}
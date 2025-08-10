// LisFS - O(1) scalable FUSE filesystem using RHC
// Demonstrates honeycomb data structure and linearizable operations

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuse3::raw::prelude::*;
use fuse3::{Result, Timestamp};
use tokio::sync::RwLock;
use bytes::Bytes;
use std::num::NonZeroU32;
use futures_util::stream::Stream;
use std::pin::Pin;

use crate::rhc::hlc::{HLC, HLCTimestamp};
use crate::rhc::leases::{LeaseManager, LeaseScope};

// Inode numbers for FUSE
type Ino = u64;
const ROOT_INO: Ino = 1;

// Block size for the filesystem
const BLOCK_SIZE: usize = 4096;

#[derive(Debug, Clone)]
struct FileMetadata {
    ino: Ino,
    name: String,
    size: u64,
    is_dir: bool,
    created: HLCTimestamp,
    modified: HLCTimestamp,
    lease_holder: Option<String>,
    blocks: Vec<BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BlockId(u64);

#[derive(Debug)]
struct ConsensusGroupInfo {
    id: String,
    location: String,
    lease_manager: Arc<LeaseManager>,
    block_storage: Arc<Mutex<HashMap<BlockId, Vec<u8>>>>,
}

/// LisFS - The O(1) scalable filesystem
pub struct LisFS {
    hlc: Arc<HLC>,
    
    // Metadata layer (global, consistent)
    files: Arc<RwLock<HashMap<Ino, FileMetadata>>>,
    inodes: Arc<Mutex<Ino>>, // Next inode number
    
    // Storage layer (distributed across consensus groups)
    consensus_groups: Arc<RwLock<Vec<ConsensusGroupInfo>>>,
    
    // Block placement (honeycomb structure)
    block_to_cg: Arc<RwLock<HashMap<BlockId, usize>>>, // Block -> CG index
    
    // Statistics for O(1) demonstration
    stats: Arc<Mutex<FSStats>>,
}

#[derive(Debug, Default)]
struct FSStats {
    read_operations: u64,
    write_operations: u64,
    metadata_lookups: u64,
    lease_acquisitions: u64,
    avg_operation_time_ns: u64,
}

impl LisFS {
    pub fn new() -> Self {
        let hlc = Arc::new(HLC::new());
        
        // Create initial consensus groups (honeycomb structure)
        let mut consensus_groups = Vec::new();
        for i in 0..3 {
            let cg_id = format!("cg-{}", i);
            let location = match i {
                0 => "Local".to_string(),
                1 => "Edge".to_string(), 
                2 => "Cloud".to_string(),
                _ => "Unknown".to_string(),
            };
            
            consensus_groups.push(ConsensusGroupInfo {
                id: cg_id.clone(),
                location,
                lease_manager: Arc::new(LeaseManager::new(cg_id, hlc.clone())),
                block_storage: Arc::new(Mutex::new(HashMap::new())),
            });
        }
        
        // Initialize with root directory
        let mut files = HashMap::new();
        files.insert(ROOT_INO, FileMetadata {
            ino: ROOT_INO,
            name: "/".to_string(),
            size: 0,
            is_dir: true,
            created: hlc.now(),
            modified: hlc.now(),
            lease_holder: Some("cg-0".to_string()),
            blocks: Vec::new(),
        });
        
        Self {
            hlc,
            files: Arc::new(RwLock::new(files)),
            inodes: Arc::new(Mutex::new(2)),
            consensus_groups: Arc::new(RwLock::new(consensus_groups)),
            block_to_cg: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(Mutex::new(FSStats::default())),
        }
    }
    
    /// O(1) block placement using honeycomb hashing
    fn place_block(&self, block_id: &BlockId) -> usize {
        // Simple hash-based placement (honeycomb structure)
        // In real implementation, would use consistent hashing for load balancing
        (block_id.0 as usize) % 3
    }
    
    /// O(1) metadata lookup
    async fn get_file_metadata(&self, ino: Ino) -> Option<FileMetadata> {
        let start = std::time::Instant::now();
        
        let files = self.files.read().await;
        let result = files.get(&ino).cloned();
        
        // Update stats
        let mut stats = self.stats.lock().unwrap();
        stats.metadata_lookups += 1;
        stats.avg_operation_time_ns = start.elapsed().as_nanos() as u64;
        
        result
    }
    
    /// O(1) block read with lease-based routing
    async fn read_block(&self, block_id: &BlockId) -> Option<Vec<u8>> {
        let start = std::time::Instant::now();
        
        // O(1) lookup of which CG holds this block
        let cg_index = {
            let block_mapping = self.block_to_cg.read().await;
            block_mapping.get(block_id).copied().unwrap_or_else(|| self.place_block(block_id))
        };
        
        // O(1) access to consensus group
        let consensus_groups = self.consensus_groups.read().await;
        if let Some(cg) = consensus_groups.get(cg_index) {
            // Local read (fast path)
            let storage = cg.block_storage.lock().unwrap();
            let result = storage.get(block_id).cloned();
            
            // Update stats
            let mut stats = self.stats.lock().unwrap();
            stats.read_operations += 1;
            stats.avg_operation_time_ns = start.elapsed().as_nanos() as u64;
            
            return result;
        }
        
        None
    }
    
    /// O(1) block write with automatic lease acquisition
    async fn write_block(&self, block_id: BlockId, data: Vec<u8>) -> Result<()> {
        let start = std::time::Instant::now();
        
        // O(1) placement decision
        let cg_index = self.place_block(&block_id);
        
        // Update block mapping
        {
            let mut block_mapping = self.block_to_cg.write().await;
            block_mapping.insert(block_id.clone(), cg_index);
        }
        
        // O(1) access to consensus group
        let consensus_groups = self.consensus_groups.read().await;
        if let Some(cg) = consensus_groups.get(cg_index) {
            // Acquire lease for this block (if needed)
            let lease_scope = LeaseScope::Block(format!("block-{}", block_id.0));
            
            match cg.lease_manager.acquire_lease(lease_scope, Duration::from_secs(60)) {
                Ok(_lease) => {
                    // We have the lease - perform local write
                    let mut storage = cg.block_storage.lock().unwrap();
                    storage.insert(block_id.clone(), data);
                    
                    println!("✅ Block write: CG-{} now holds lease for block {}", 
                             cg_index, block_id.0);
                }
                Err(e) => {
                    println!("⚠️ Lease acquisition failed: {:?}", e);
                    return Err(libc::EIO.into());
                }
            }
        }
        
        // Update stats
        let mut stats = self.stats.lock().unwrap();
        stats.write_operations += 1;
        stats.lease_acquisitions += 1;
        stats.avg_operation_time_ns = start.elapsed().as_nanos() as u64;
        
        Ok(())
    }
    
    /// Generate next inode number
    fn next_ino(&self) -> Ino {
        let mut inodes = self.inodes.lock().unwrap();
        let ino = *inodes;
        *inodes += 1;
        ino
    }
    
    /// Print filesystem statistics (demonstrates O(1) scaling)
    pub async fn print_stats(&self) {
        let stats = self.stats.lock().unwrap();
        let files = self.files.read().await;
        let consensus_groups = self.consensus_groups.read().await;
        
        println!("\n=== LisFS Performance Statistics ===");
        println!("Files: {} (O(1) metadata lookup)", files.len());
        println!("Consensus Groups: {} (honeycomb structure)", consensus_groups.len());
        println!("Read Operations: {} (avg: {}ns)", stats.read_operations, stats.avg_operation_time_ns);
        println!("Write Operations: {} (lease-based)", stats.write_operations);
        println!("Metadata Lookups: {} (constant time)", stats.metadata_lookups);
        println!("Lease Acquisitions: {} (distributed)", stats.lease_acquisitions);
        
        // Show block distribution across CGs
        println!("\nBlock Distribution (Honeycomb):");
        for (i, cg) in consensus_groups.iter().enumerate() {
            let block_count = cg.block_storage.lock().unwrap().len();
            println!("  CG-{} ({}): {} blocks", i, cg.location, block_count);
        }
    }
}

// FUSE filesystem implementation
impl Filesystem for LisFS {
    // Required associated types using proper stream types
    type DirEntryStream<'a> = Pin<Box<dyn Stream<Item = Result<DirectoryEntry>> + Send + 'a>> where Self: 'a;
    type DirEntryPlusStream<'a> = Pin<Box<dyn Stream<Item = Result<DirectoryEntryPlus>> + Send + 'a>> where Self: 'a;

    async fn init(&self, _req: Request) -> Result<ReplyInit> {
        Ok(ReplyInit { max_write: NonZeroU32::new(128 * 1024).unwrap() })
    }

    async fn destroy(&self, _req: Request) {
        // Cleanup - nothing needed for our demo
    }

    async fn getattr(&self, _req: Request, ino: u64, _fh: Option<u64>, _flags: u32) -> Result<ReplyAttr> {
        if let Some(metadata) = self.get_file_metadata(ino).await {
            let timestamp = Timestamp { sec: (metadata.modified.physical / 1000) as i64, nsec: 0 };
            let attr = FileAttr {
                ino,
                size: metadata.size,
                blocks: (metadata.size + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64,
                atime: timestamp,
                mtime: timestamp,
                ctime: timestamp,
                kind: if metadata.is_dir { FileType::Directory } else { FileType::RegularFile },
                perm: if metadata.is_dir { 0o755 } else { 0o644 },
                nlink: 1,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                blksize: BLOCK_SIZE as u32,
            };
            
            Ok(ReplyAttr { 
                attr, 
                ttl: Duration::from_secs(1) 
            })
        } else {
            Err(libc::ENOENT.into())
        }
    }
    
    async fn lookup(&self, _req: Request, parent: u64, name: &OsStr) -> Result<ReplyEntry> {
        let name_str = name.to_string_lossy();
        println!("🔍 Lookup: parent={}, name={}", parent, name_str);
        
        // For demo, create files on demand
        if parent == ROOT_INO && name_str != "." && name_str != ".." {
            let ino = self.next_ino();
            let metadata = FileMetadata {
                ino,
                name: name_str.to_string(),
                size: 0,
                is_dir: false,
                created: self.hlc.now(),
                modified: self.hlc.now(),
                lease_holder: None,
                blocks: Vec::new(),
            };
            
            // O(1) insertion into metadata layer
            {
                let mut files = self.files.write().await;
                files.insert(ino, metadata);
            }
            
            println!("📄 Created file: {} (ino: {})", name_str, ino);
            
            let timestamp = Timestamp { sec: 0, nsec: 0 };
            let attr = FileAttr {
                ino,
                size: 0,
                blocks: 0,
                atime: timestamp,
                mtime: timestamp, 
                ctime: timestamp,
                kind: FileType::RegularFile,
                perm: 0o644,
                nlink: 1,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                blksize: BLOCK_SIZE as u32,
            };
            
            return Ok(ReplyEntry {
                attr,
                ttl: Duration::from_secs(1),
                generation: 0,
            });
        }
        
        Err(libc::ENOENT.into())
    }
    
    async fn read(&self, _req: Request, ino: u64, _fh: u64, offset: u64, size: u32) -> Result<ReplyData> {
        println!("📖 Read: ino={}, offset={}, size={}", ino, offset, size);
        
        if let Some(_metadata) = self.get_file_metadata(ino).await {
            // Calculate which blocks we need
            let start_block = (offset as u64) / BLOCK_SIZE as u64;
            let end_block = ((offset as u64 + size as u64 - 1) / BLOCK_SIZE as u64) + 1;
            
            let mut result_data = Vec::new();
            
            // O(1) block reads
            for block_idx in start_block..end_block {
                let block_id = BlockId(ino * 1000 + block_idx);
                
                if let Some(block_data) = self.read_block(&block_id).await {
                    result_data.extend_from_slice(&block_data);
                } else {
                    // Block doesn't exist - return zeros
                    result_data.extend_from_slice(&vec![0u8; BLOCK_SIZE]);
                }
            }
            
            // Trim to requested range
            let file_offset = (offset as usize) % BLOCK_SIZE;
            let end_pos = std::cmp::min(file_offset + size as usize, result_data.len());
            
            if file_offset < result_data.len() {
                let data = result_data[file_offset..end_pos].to_vec();
                println!("📖 Read {} bytes from ino {}", data.len(), ino);
                Ok(ReplyData { data: Bytes::from(data) })
            } else {
                Ok(ReplyData { data: Bytes::new() })
            }
        } else {
            Err(libc::ENOENT.into())
        }
    }
    
    async fn write(&self, _req: Request, ino: u64, _fh: u64, offset: u64, data: &[u8], _write_flags: u32, _flags: u32) -> Result<ReplyWrite> {
        println!("✏️ Write: ino={}, offset={}, size={}", ino, offset, data.len());
        
        // Calculate which blocks we're writing to
        let start_block = (offset as u64) / BLOCK_SIZE as u64;
        let end_block = ((offset as u64 + data.len() as u64 - 1) / BLOCK_SIZE as u64) + 1;
        
        // O(1) block writes with automatic lease acquisition
        let mut written = 0;
        for block_idx in start_block..end_block {
            let block_id = BlockId(ino * 1000 + block_idx);
            let block_offset = ((offset as u64) + written) % BLOCK_SIZE as u64;
            let block_end = std::cmp::min(block_offset + (data.len() as u64 - written), BLOCK_SIZE as u64);
            
            // Read existing block if partial write
            let mut block_data = if block_offset > 0 || block_end < BLOCK_SIZE as u64 {
                self.read_block(&block_id).await.unwrap_or_else(|| vec![0u8; BLOCK_SIZE])
            } else {
                vec![0u8; BLOCK_SIZE]
            };
            
            // Update the block with new data
            let data_start = written as usize;
            let data_end = std::cmp::min(data_start + (block_end - block_offset) as usize, data.len());
            block_data[block_offset as usize..block_end as usize]
                .copy_from_slice(&data[data_start..data_end]);
            
            // O(1) write with lease acquisition
            match self.write_block(block_id.clone(), block_data).await {
                Ok(_) => {
                    written += data_end as u64 - data_start as u64;
                }
                Err(e) => {
                    println!("❌ Write failed: {:?}", e);
                    return Err(e);
                }
            }
        }
        
        // Update file metadata
        {
            let mut files = self.files.write().await;
            if let Some(metadata) = files.get_mut(&ino) {
                metadata.size = std::cmp::max(metadata.size, offset as u64 + data.len() as u64);
                metadata.modified = self.hlc.now();
            }
        }
        
        println!("✅ Wrote {} bytes to ino {}", data.len(), ino);
        Ok(ReplyWrite { written: data.len() as u32 })
    }
}

// Helper trait implementations  
impl From<HLCTimestamp> for SystemTime {
    fn from(hlc: HLCTimestamp) -> Self {
        UNIX_EPOCH + Duration::from_millis(hlc.physical)
    }
}
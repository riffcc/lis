// Variable Block Size Storage with bitvec tracking and null compression
// Demonstrates adaptive block sizing for different workloads:
// - Small files (trillions of files): 512B blocks
// - VM workloads: 4KB blocks  
// - Large media: 64KB blocks
// - Sparse data: Variable with null compression

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bitvec::prelude::*;
use lis::rhc::hlc::{HLC, HLCTimestamp};
use lis::rhc::leases::{LeaseManager, LeaseScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockSize {
    Tiny = 512,      // Trillions of small files (metadata, configs)
    Small = 1024,    // Small documents
    Standard = 4096, // VM workloads, general purpose
    Large = 16384,   // Media files
    Huge = 65536,    // Large media, streaming
}

impl BlockSize {
    fn from_data_pattern(data: &[u8], file_extension: &str) -> Self {
        // Adaptive block size based on data patterns and file type
        match file_extension {
            "vm" | "qcow2" | "vdi" | "vmdk" => BlockSize::Standard, // VM images
            "mp4" | "mkv" | "avi" | "mov" | "webm" => BlockSize::Huge, // Video files
            "mp3" | "flac" | "wav" | "ogg" => BlockSize::Large, // Audio files
            "jpg" | "png" | "tiff" | "bmp" => BlockSize::Large, // Images
            "txt" | "json" | "yaml" | "toml" | "conf" => BlockSize::Tiny, // Config files
            _ => {
                // Analyze data content for optimal block size
                if data.len() < 2048 {
                    BlockSize::Tiny
                } else if data.len() < 8192 {
                    BlockSize::Small
                } else if has_sparse_pattern(data) {
                    BlockSize::Standard // Good for null compression
                } else if data.len() > 1024 * 1024 {
                    BlockSize::Large // Large files benefit from bigger blocks
                } else {
                    BlockSize::Standard
                }
            }
        }
    }
    
    fn as_usize(self) -> usize {
        self as usize
    }
}

/// Check if data has sparse patterns (lots of zeros/repeated bytes)
fn has_sparse_pattern(data: &[u8]) -> bool {
    if data.len() < 1024 { return false; }
    
    let sample_size = std::cmp::min(1024, data.len());
    let mut zero_count = 0;
    let mut repeated_byte_count = 0;
    let mut prev_byte = data[0];
    
    for &byte in data.iter().take(sample_size) {
        if byte == 0 {
            zero_count += 1;
        }
        if byte == prev_byte {
            repeated_byte_count += 1;
        }
        prev_byte = byte;
    }
    
    // Sparse if >30% zeros or >50% repeated bytes
    zero_count as f32 / sample_size as f32 > 0.3 || 
    repeated_byte_count as f32 / sample_size as f32 > 0.5
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VariableBlockId {
    file_hash: u64,
    block_index: u64,
    block_size: BlockSize,
}

#[derive(Debug, Clone)]
struct CompressedBlock {
    data: Vec<u8>,
    original_size: usize,
    compression_ratio: f32,
    null_bitmap: Option<BitVec>, // Track null/zero regions
}

impl CompressedBlock {
    fn new(data: Vec<u8>, _block_size: BlockSize) -> Self {
        let original_size = data.len();
        let (compressed_data, null_bitmap) = null_compress(&data);
        let compression_ratio = compressed_data.len() as f32 / original_size as f32;
        
        Self {
            data: compressed_data,
            original_size,
            compression_ratio,
            null_bitmap,
        }
    }
    
    fn decompress(&self) -> Vec<u8> {
        null_decompress(&self.data, self.original_size, &self.null_bitmap)
    }
}

/// Null compression: remove zero regions and track them in bitmap
fn null_compress(data: &[u8]) -> (Vec<u8>, Option<BitVec>) {
    if data.iter().all(|&b| b != 0) {
        // No zeros, no compression needed
        return (data.to_vec(), None);
    }
    
    let mut compressed = Vec::new();
    let mut null_bitmap = bitvec![0; data.len()];
    
    for (i, &byte) in data.iter().enumerate() {
        if byte == 0 {
            null_bitmap.set(i, true);
        } else {
            compressed.push(byte);
        }
    }
    
    // Only use compression if it saves significant space
    if compressed.len() < data.len() * 3 / 4 {
        (compressed, Some(null_bitmap))
    } else {
        (data.to_vec(), None)
    }
}

/// Null decompression: restore zero regions using bitmap
fn null_decompress(compressed: &[u8], original_size: usize, null_bitmap: &Option<BitVec>) -> Vec<u8> {
    match null_bitmap {
        None => compressed.to_vec(),
        Some(bitmap) => {
            let mut result = vec![0u8; original_size];
            let mut compressed_idx = 0;
            
            for (i, is_null) in bitmap.iter().enumerate() {
                if i >= original_size { break; }
                
                if *is_null {
                    result[i] = 0;
                } else {
                    if compressed_idx < compressed.len() {
                        result[i] = compressed[compressed_idx];
                        compressed_idx += 1;
                    }
                }
            }
            result
        }
    }
}

#[derive(Debug, Clone)]
struct FileMetadata {
    name: String,
    size: u64,
    created: HLCTimestamp,
    modified: HLCTimestamp,
    blocks: Vec<VariableBlockId>,
    // Bitvec track for block size changes over time
    blocksize_track: BitVec,
    current_block_size: BlockSize,
}

/// Block size tracking using efficient bitflips
/// Format: 3-bit encoding for block sizes:
/// 000 = Tiny (512B)
/// 001 = Small (1KB) 
/// 010 = Standard (4KB)
/// 011 = Large (16KB)
/// 100 = Huge (64KB)
#[derive(Debug, Clone)]
struct BlockSizeTracker {
    size_changes: BitVec,
    change_positions: Vec<usize>, // Where changes occur
}

impl BlockSizeTracker {
    fn new(initial_size: BlockSize) -> Self {
        let mut tracker = Self {
            size_changes: BitVec::new(),
            change_positions: Vec::new(),
        };
        tracker.encode_block_size(initial_size);
        tracker
    }
    
    fn encode_block_size(&mut self, size: BlockSize) -> usize {
        let size_bits = match size {
            BlockSize::Tiny => bitvec![0, 0, 0],
            BlockSize::Small => bitvec![0, 0, 1],
            BlockSize::Standard => bitvec![0, 1, 0],
            BlockSize::Large => bitvec![0, 1, 1],
            BlockSize::Huge => bitvec![1, 0, 0],
        };
        
        let pos = self.size_changes.len();
        self.size_changes.extend_from_bitslice(&size_bits);
        self.change_positions.push(pos);
        pos
    }
    
    fn change_block_size(&mut self, new_size: BlockSize) {
        self.encode_block_size(new_size);
        println!("📊 Block size changed to {:?} at position {}", new_size, self.size_changes.len() - 3);
    }
    
    fn get_block_size_at(&self, position: usize) -> BlockSize {
        // Find the most recent block size change before or at this position
        let change_pos = self.change_positions.iter()
            .rev()
            .find(|&&pos| pos <= position)
            .copied()
            .unwrap_or(0);
        
        if change_pos + 3 <= self.size_changes.len() {
            let size_bits = &self.size_changes[change_pos..change_pos + 3];
            match (size_bits[0], size_bits[1], size_bits[2]) {
                (false, false, false) => BlockSize::Tiny,
                (false, false, true) => BlockSize::Small,
                (false, true, false) => BlockSize::Standard,
                (false, true, true) => BlockSize::Large,
                (true, false, false) => BlockSize::Huge,
                _ => BlockSize::Standard, // Default fallback
            }
        } else {
            BlockSize::Standard
        }
    }
}

struct ConsensusGroup {
    id: String,
    location: String,
    lease_manager: Arc<LeaseManager>,
    block_storage: Arc<Mutex<HashMap<VariableBlockId, CompressedBlock>>>,
}

impl ConsensusGroup {
    fn new(id: String, location: String, hlc: Arc<HLC>) -> Self {
        Self {
            id: id.clone(),
            location,
            lease_manager: Arc::new(LeaseManager::new(id, hlc)),
            block_storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    fn write_block(&self, block_id: VariableBlockId, data: Vec<u8>) -> Result<CompressedBlock, String> {
        let lease_scope = LeaseScope::Block(format!("block-{}-{}", block_id.file_hash, block_id.block_index));
        
        match self.lease_manager.acquire_lease(lease_scope, Duration::from_secs(60)) {
            Ok(_lease) => {
                let compressed_block = CompressedBlock::new(data, block_id.block_size);
                println!("💾 Block {}-{} compressed {:.1}x in CG-{} ({})", 
                         block_id.file_hash, block_id.block_index, 
                         1.0 / compressed_block.compression_ratio,
                         &self.id, self.location);
                
                let mut storage = self.block_storage.lock().unwrap();
                storage.insert(block_id, compressed_block.clone());
                Ok(compressed_block)
            }
            Err(e) => Err(format!("Lease acquisition failed: {:?}", e))
        }
    }
    
    fn read_block(&self, block_id: &VariableBlockId) -> Option<Vec<u8>> {
        let storage = self.block_storage.lock().unwrap();
        storage.get(block_id).map(|compressed_block| compressed_block.decompress())
    }
    
    fn block_count(&self) -> usize {
        self.block_storage.lock().unwrap().len()
    }
    
    fn compression_stats(&self) -> (usize, f32) {
        let storage = self.block_storage.lock().unwrap();
        let total_blocks = storage.len();
        let avg_compression = if total_blocks > 0 {
            storage.values().map(|b| b.compression_ratio).sum::<f32>() / total_blocks as f32
        } else {
            1.0
        };
        (total_blocks, avg_compression)
    }
}

/// Variable Block Size Storage System
struct VariableBlockStorage {
    hlc: Arc<HLC>,
    consensus_groups: Vec<ConsensusGroup>,
    files: Arc<Mutex<HashMap<String, FileMetadata>>>,
    // Global block size tracker for the filesystem
    block_size_tracker: Arc<Mutex<BlockSizeTracker>>,
    stats: Arc<Mutex<StorageStats>>,
}

#[derive(Debug, Default)]
struct StorageStats {
    reads: u64,
    writes: u64,
    block_size_changes: u64,
    total_compression_ratio: f32,
    avg_operation_time_ns: u64,
}

impl VariableBlockStorage {
    fn new() -> Self {
        let hlc = Arc::new(HLC::new());
        
        let consensus_groups = vec![
            ConsensusGroup::new("cg-ssd".to_string(), "NVMe SSD".to_string(), hlc.clone()),
            ConsensusGroup::new("cg-hdd".to_string(), "Spinning Disk".to_string(), hlc.clone()),
            ConsensusGroup::new("cg-cloud".to_string(), "Cloud Storage".to_string(), hlc.clone()),
        ];
        
        Self {
            hlc,
            consensus_groups,
            files: Arc::new(Mutex::new(HashMap::new())),
            block_size_tracker: Arc::new(Mutex::new(BlockSizeTracker::new(BlockSize::Standard))),
            stats: Arc::new(Mutex::new(StorageStats::default())),
        }
    }
    
    fn place_block(&self, block_id: &VariableBlockId) -> usize {
        // Placement strategy based on block size and access patterns
        match block_id.block_size {
            BlockSize::Tiny | BlockSize::Small => 0, // Fast SSD for small files
            BlockSize::Standard => (block_id.file_hash as usize) % 2, // SSD or HDD
            BlockSize::Large | BlockSize::Huge => 2, // Cloud for large files
        }
    }
    
    fn write(&self, filename: &str, data: Vec<u8>) -> Result<(), String> {
        let start = Instant::now();
        
        // Extract file extension for block size heuristics
        let extension = filename.split('.').last().unwrap_or("");
        let optimal_block_size = BlockSize::from_data_pattern(&data, extension);
        
        println!("📝 Writing file '{}' ({} bytes) with {:?} blocks", 
                 filename, data.len(), optimal_block_size);
        
        // Update global block size tracker if size changed
        {
            let mut tracker = self.block_size_tracker.lock().unwrap();
            let current_size = tracker.get_block_size_at(tracker.size_changes.len().saturating_sub(1));
            if current_size != optimal_block_size {
                tracker.change_block_size(optimal_block_size);
                let mut stats = self.stats.lock().unwrap();
                stats.block_size_changes += 1;
            }
        }
        
        let block_size = optimal_block_size.as_usize();
        let file_hash = self.hash_filename(filename);
        let blocks_needed = (data.len() + block_size - 1) / block_size;
        
        let mut block_ids = Vec::new();
        let mut total_compression_ratio = 0.0;
        
        // Write variable-sized blocks
        for i in 0..blocks_needed {
            let start_byte = i * block_size;
            let end_byte = std::cmp::min((i + 1) * block_size, data.len());
            let block_data = data[start_byte..end_byte].to_vec();
            
            let block_id = VariableBlockId {
                file_hash,
                block_index: i as u64,
                block_size: optimal_block_size,
            };
            
            let cg_index = self.place_block(&block_id);
            
            match self.consensus_groups[cg_index].write_block(block_id.clone(), block_data) {
                Ok(compressed_block) => {
                    total_compression_ratio += compressed_block.compression_ratio;
                    block_ids.push(block_id);
                }
                Err(e) => {
                    println!("❌ Block write failed: {}", e);
                    return Err(e);
                }
            }
        }
        
        // Create file metadata with block size tracking
        let metadata = FileMetadata {
            name: filename.to_string(),
            size: data.len() as u64,
            created: self.hlc.now(),
            modified: self.hlc.now(),
            blocks: block_ids,
            blocksize_track: {
                let tracker = self.block_size_tracker.lock().unwrap();
                tracker.size_changes.clone()
            },
            current_block_size: optimal_block_size,
        };
        
        {
            let mut files = self.files.lock().unwrap();
            files.insert(filename.to_string(), metadata);
        }
        
        // Update stats
        let elapsed = start.elapsed();
        {
            let mut stats = self.stats.lock().unwrap();
            stats.writes += 1;
            stats.total_compression_ratio = total_compression_ratio / blocks_needed as f32;
            stats.avg_operation_time_ns = elapsed.as_nanos() as u64;
        }
        
        println!("✅ File '{}' written with {:.1}x compression in {:?}", 
                 filename, blocks_needed as f32 / total_compression_ratio, elapsed);
        Ok(())
    }
    
    fn read(&self, filename: &str) -> Result<Vec<u8>, String> {
        let start = Instant::now();
        
        println!("📖 Reading variable-block file '{}'", filename);
        
        let metadata = {
            let files = self.files.lock().unwrap();
            files.get(filename).cloned()
        };
        
        let metadata = metadata.ok_or_else(|| "File not found".to_string())?;
        
        println!("  Found {} blocks (size: {:?})", metadata.blocks.len(), metadata.current_block_size);
        
        let mut result_data = Vec::new();
        
        for block_id in &metadata.blocks {
            let cg_index = self.place_block(block_id);
            
            if let Some(block_data) = self.consensus_groups[cg_index].read_block(block_id) {
                println!("  Block {}-{} ← {} ({} bytes)", 
                         block_id.file_hash, block_id.block_index, 
                         self.consensus_groups[cg_index].location, block_data.len());
                result_data.extend(block_data);
            } else {
                return Err(format!("Block {}-{} not found", block_id.file_hash, block_id.block_index));
            }
        }
        
        let elapsed = start.elapsed();
        {
            let mut stats = self.stats.lock().unwrap();
            stats.reads += 1;
            stats.avg_operation_time_ns = elapsed.as_nanos() as u64;
        }
        
        println!("✅ File '{}' read in {:?} ({} bytes)", filename, elapsed, result_data.len());
        Ok(result_data)
    }
    
    fn hash_filename(&self, filename: &str) -> u64 {
        filename.bytes().map(|b| b as u64).sum()
    }
    
    fn print_stats(&self) {
        let stats = self.stats.lock().unwrap();
        let files = self.files.lock().unwrap();
        
        println!("\n=== Variable Block Size Storage Statistics ===");
        println!("Files: {} (adaptive block sizing)", files.len());
        println!("Read Operations: {} (avg: {}ns)", stats.reads, stats.avg_operation_time_ns);
        println!("Write Operations: {} (avg compression: {:.1}x)", stats.writes, 1.0 / stats.total_compression_ratio);
        println!("Block Size Changes: {} (bitvec tracking)", stats.block_size_changes);
        
        println!("\nStorage Distribution by Block Size:");
        for cg in self.consensus_groups.iter() {
            let (block_count, avg_compression) = cg.compression_stats();
            println!("  {}: {} blocks (avg compression: {:.1}x)", 
                     cg.location, block_count, 1.0 / avg_compression);
        }
        
        println!("\nBlock Size Analysis:");
        for (filename, metadata) in files.iter() {
            println!("  '{}': {:?} blocks ({} total)", 
                     filename, metadata.current_block_size, metadata.blocks.len());
        }
        
        // Show bitvec efficiency
        let tracker = self.block_size_tracker.lock().unwrap();
        println!("\nBitvec Tracking Efficiency:");
        println!("  Total bits used: {} (for {} size changes)", 
                 tracker.size_changes.len(), tracker.change_positions.len());
        println!("  Bits per change: {:.1}", 
                 tracker.size_changes.len() as f32 / tracker.change_positions.len() as f32);
    }
}

fn main() {
    println!("=== Variable Block Size Storage Demo ===");
    println!("Demonstrates adaptive block sizing with bitvec tracking and null compression\n");
    
    let storage = VariableBlockStorage::new();
    
    // Test files with different patterns optimized for different block sizes
    let test_data = vec![
        // Small config files -> Tiny blocks
        ("config.toml", b"debug = true\nport = 8080\nhost = \"localhost\"".to_vec()),
        ("package.json", br#"{"name": "test", "version": "1.0.0"}"#.to_vec()),
        
        // VM image simulation -> Standard blocks
        ("vm_disk.vm", vec![0xAA; 8192]), // VM boot sector pattern
        
        // Sparse file -> Good for null compression
        ("sparse_log.txt", {
            let mut data = vec![0u8; 16384];
            // Add some actual data in between zeros
            data[1000..1100].fill(b'L'); // Log entries
            data[5000..5200].fill(b'E'); // Error messages
            data[10000..10050].fill(b'W'); // Warnings
            data
        }),
        
        // Large media file -> Huge blocks
        ("video.mp4", vec![0x42; 131072]), // 128KB video data
        
        // Small text files (trillions of files scenario)
        ("readme.txt", b"This is a small readme file for testing.".to_vec()),
        ("license.txt", b"MIT License...".to_vec()),
    ];
    
    println!("--- Writing Files with Adaptive Block Sizing ---");
    for (filename, data) in &test_data {
        match storage.write(filename, data.clone()) {
            Ok(_) => println!("✓ {} written", filename),
            Err(e) => println!("✗ {} failed: {}", filename, e),
        }
        println!();
    }
    
    storage.print_stats();
    
    println!("\n--- Reading Files ---");
    for (filename, original_data) in &test_data {
        match storage.read(filename) {
            Ok(data) => {
                if data == *original_data {
                    println!("✓ {} read successfully (verified)", filename);
                } else {
                    println!("✗ {} data mismatch!", filename);
                }
            }
            Err(e) => println!("✗ {} read failed: {}", filename, e),
        }
        println!();
    }
    
    storage.print_stats();
    
    println!("\n=== Variable Block Size Benefits ===");
    println!("✓ Adaptive sizing: Optimal blocks for each workload type");
    println!("✓ Null compression: Sparse data automatically compressed");
    println!("✓ Bitvec tracking: Efficient block size change history");
    println!("✓ Workload optimization: VM, media, and tiny file support");
    println!("✓ Zero padding removal: Significant space savings");
    
    println!("\n🚀 This optimizes for:");
    println!("  • Trillions of files: 512B blocks for configs/metadata");
    println!("  • VM workloads: 4KB blocks matching page sizes");
    println!("  • Large media: 64KB blocks for streaming efficiency");
    println!("  • Sparse data: Variable blocks with null compression");
    println!("  • Mixed workloads: Dynamic adaptation over time");
}
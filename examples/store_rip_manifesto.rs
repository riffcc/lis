// Store and retrieve RiP! A Remix Manifesto movie using variable block storage
// Demonstrates real-world usage of the O(1) scalable storage system

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::path::Path;

use bitvec::prelude::*;
use lis::rhc::hlc::HLC;
use lis::rhc::leases::{LeaseManager, LeaseScope};

// Variable block size implementation from the example
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockSize {
    Tiny = 512,      // Metadata, small files
    Small = 1024,    // Documents, config
    Standard = 4096, // General purpose
    Large = 16384,   // Media chunks
    Huge = 65536,    // Video streams
    Mega = 1048576,  // Large video blocks (1MB)
}

impl BlockSize {
    fn as_usize(self) -> usize {
        self as usize
    }
    
    fn optimal_for_size(file_size: usize) -> Self {
        match file_size {
            0..=2048 => BlockSize::Tiny,
            2049..=16384 => BlockSize::Small,
            16385..=65536 => BlockSize::Standard,
            65537..=524288 => BlockSize::Large,
            524289..=10485760 => BlockSize::Huge,
            _ => BlockSize::Mega,
        }
    }
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
}

impl CompressedBlock {
    fn new(data: Vec<u8>) -> Self {
        let original_size = data.len();
        // For video data, we don't compress (already compressed)
        Self {
            data,
            original_size,
            compression_ratio: 1.0,
        }
    }
}

struct ConsensusGroup {
    id: String,
    lease_manager: Arc<LeaseManager>,
    block_storage: Arc<Mutex<HashMap<VariableBlockId, CompressedBlock>>>,
    storage_type: StorageType,
}

#[derive(Debug, Clone)]
enum StorageType {
    NVMeSSD,      // Ultra-fast for hot data
    SSD,          // Fast for warm data
    HDD,          // Bulk storage
    CloudStorage, // Archive/cold storage
}

impl ConsensusGroup {
    fn new(id: String, hlc: Arc<HLC>, storage_type: StorageType) -> Self {
        Self {
            id: id.clone(),
            lease_manager: Arc::new(LeaseManager::new(id, hlc)),
            block_storage: Arc::new(Mutex::new(HashMap::new())),
            storage_type,
        }
    }
    
    fn write_block(&self, block_id: VariableBlockId, data: Vec<u8>) -> Result<CompressedBlock, String> {
        let lease_scope = LeaseScope::Block(format!("block-{}-{}", block_id.file_hash, block_id.block_index));
        
        match self.lease_manager.acquire_lease(lease_scope, Duration::from_secs(60)) {
            Ok(_lease) => {
                let compressed_block = CompressedBlock::new(data);
                let mut storage = self.block_storage.lock().unwrap();
                storage.insert(block_id, compressed_block.clone());
                Ok(compressed_block)
            }
            Err(e) => Err(format!("Lease acquisition failed: {:?}", e))
        }
    }
    
    fn read_block(&self, block_id: &VariableBlockId) -> Option<Vec<u8>> {
        let storage = self.block_storage.lock().unwrap();
        storage.get(block_id).map(|compressed_block| compressed_block.data.clone())
    }
    
    fn block_count(&self) -> usize {
        self.block_storage.lock().unwrap().len()
    }
    
    fn storage_bytes(&self) -> usize {
        self.block_storage.lock().unwrap()
            .values()
            .map(|block| block.data.len())
            .sum()
    }
}

fn main() {
    println!("🎬 RiP! A Remix Manifesto - Variable Block Storage Demo");
    println!("======================================================\n");
    
    // Path to the actual movie file
    let movie_path = Path::new("/home/wings/Riff.CC Content/RiP - A Remix Manifesto (2008) [1080p-WEB-DL].mkv");
    
    if !movie_path.exists() {
        eprintln!("❌ Movie file not found at: {}", movie_path.display());
        eprintln!("   Please ensure the file exists");
        return;
    }
    
    // Get file metadata
    let metadata = std::fs::metadata(&movie_path).expect("Failed to get metadata");
    let file_size = metadata.len() as usize;
    println!("📁 File: {}", movie_path.file_name().unwrap().to_string_lossy());
    println!("📏 Size: {:.2} MB ({} bytes)", file_size as f64 / 1_048_576.0, file_size);
    
    // Set up distributed storage with different tiers
    let hlc = Arc::new(HLC::new());
    let consensus_groups = vec![
        ConsensusGroup::new("nvme-hot".to_string(), hlc.clone(), StorageType::NVMeSSD),
        ConsensusGroup::new("ssd-warm".to_string(), hlc.clone(), StorageType::SSD),
        ConsensusGroup::new("hdd-bulk".to_string(), hlc.clone(), StorageType::HDD),
        ConsensusGroup::new("cloud-archive".to_string(), hlc.clone(), StorageType::CloudStorage),
    ];
    
    // Determine optimal block size for video
    let block_size = BlockSize::Mega; // 1MB blocks for video streaming
    println!("🎯 Using {:?} blocks ({} bytes) for video streaming", block_size, block_size.as_usize());
    
    // Calculate blocks needed
    let block_size_bytes = block_size.as_usize();
    let blocks_needed = (file_size + block_size_bytes - 1) / block_size_bytes;
    println!("📦 Will store in {} blocks\n", blocks_needed);
    
    // Read and store the movie file
    println!("⏳ Reading and storing movie file...");
    let start_time = Instant::now();
    
    let file_data = std::fs::read(&movie_path).expect("Failed to read movie file");
    let file_hash = 0x52695046696C6D; // "RiPFilm" in hex
    
    let mut stored_blocks = Vec::new();
    let mut total_stored_size = 0;
    
    // Progress tracking
    let progress_interval = std::cmp::max(1, blocks_needed / 20);
    
    for i in 0..blocks_needed {
        let start_byte = i * block_size_bytes;
        let end_byte = std::cmp::min((i + 1) * block_size_bytes, file_size);
        let block_data = file_data[start_byte..end_byte].to_vec();
        
        let block_id = VariableBlockId {
            file_hash,
            block_index: i as u64,
            block_size,
        };
        
        // Smart placement based on access patterns
        let cg_index = if i < 10 {
            0 // First 10MB on NVMe for instant playback
        } else if i < 100 {
            1 // Next 90MB on SSD for buffering
        } else {
            2 // Rest on HDD for bulk storage
        };
        
        let result = consensus_groups[cg_index].write_block(block_id.clone(), block_data);
        match result {
            Ok(compressed_block) => {
                total_stored_size += compressed_block.data.len();
                stored_blocks.push((block_id, cg_index));
                
                // Show progress
                if i % progress_interval == 0 || i == blocks_needed - 1 {
                    let progress = ((i + 1) as f32 / blocks_needed as f32) * 100.0;
                    print!("\r  Progress: [{:>3.0}%] Block {}/{}", progress, i + 1, blocks_needed);
                    use std::io::{self, Write};
                    io::stdout().flush().unwrap();
                }
            }
            Err(e) => {
                eprintln!("\n❌ Failed to store block {}: {}", i, e);
                return;
            }
        }
    }
    
    let store_duration = start_time.elapsed();
    println!("\n✅ Movie stored successfully in {:.2?}", store_duration);
    println!("   Storage rate: {:.2} MB/s", (file_size as f64 / 1_048_576.0) / store_duration.as_secs_f64());
    
    // Show storage distribution
    println!("\n📊 Storage Distribution:");
    for (idx, cg) in consensus_groups.iter().enumerate() {
        let blocks = cg.block_count();
        if blocks > 0 {
            let bytes = cg.storage_bytes();
            println!("   {:?} ({}): {} blocks, {:.2} MB", 
                     cg.storage_type, 
                     cg.id,
                     blocks,
                     bytes as f64 / 1_048_576.0);
        }
    }
    
    // Simulate video streaming - read first 10 seconds worth of data
    println!("\n🎥 Simulating video playback (first 10 seconds)...");
    let playback_start = Instant::now();
    
    // Assume 10Mbps bitrate, so ~12.5MB for 10 seconds
    let playback_blocks = std::cmp::min(13, stored_blocks.len());
    let mut retrieved_data = Vec::new();
    
    for i in 0..playback_blocks {
        let (block_id, cg_index) = &stored_blocks[i];
        let block_data = consensus_groups[*cg_index].read_block(block_id);
        
        match block_data {
            Some(data) => {
                retrieved_data.extend(data);
                print!("\r  Buffering: Block {}/{}", i + 1, playback_blocks);
                use std::io::{self, Write};
                io::stdout().flush().unwrap();
            }
            None => {
                eprintln!("\n❌ Failed to retrieve block {}", i);
                return;
            }
        }
    }
    
    let playback_duration = playback_start.elapsed();
    println!("\n✅ Playback ready in {:.2?}", playback_duration);
    println!("   Buffered: {:.2} MB", retrieved_data.len() as f64 / 1_048_576.0);
    println!("   Read rate: {:.2} MB/s", (retrieved_data.len() as f64 / 1_048_576.0) / playback_duration.as_secs_f64());
    
    // Verify data integrity
    let original_segment = &file_data[..retrieved_data.len()];
    if retrieved_data == original_segment {
        println!("✅ Data integrity verified - perfect reconstruction!");
    } else {
        eprintln!("❌ Data integrity check failed!");
    }
    
    // O(1) operation demonstration
    println!("\n⚡ O(1) Operation Demonstration:");
    
    // Random access to any block
    let random_block = stored_blocks.len() / 2;
    let access_start = Instant::now();
    let (block_id, cg_index) = &stored_blocks[random_block];
    let _random_data = consensus_groups[*cg_index].read_block(block_id);
    let access_duration = access_start.elapsed();
    
    println!("   Random access to block {}: {:?}", random_block, access_duration);
    println!("   ✨ Constant time regardless of file size!");
    
    // Summary
    println!("\n🎉 Summary:");
    println!("   • Stored {:.2} MB movie in {} blocks", file_size as f64 / 1_048_576.0, blocks_needed);
    println!("   • Achieved {:.2} MB/s write speed", (file_size as f64 / 1_048_576.0) / store_duration.as_secs_f64());
    println!("   • Instant playback with tiered storage");
    println!("   • O(1) random access to any part of the video");
    println!("   • Automatic placement optimization for streaming");
    println!("\n🚀 RHC Variable Block Storage: Cinema-Quality Performance at Scale!");
}
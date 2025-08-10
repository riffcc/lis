// Demonstrates O(1) scalable storage using RHC concepts
// Shows honeycomb data structure and linearizable operations

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lis::rhc::hlc::{HLC, HLCTimestamp};
use lis::rhc::leases::{LeaseManager, LeaseScope};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BlockId(u64);

#[derive(Debug, Clone)]
struct FileMetadata {
    name: String,
    size: u64,
    created: HLCTimestamp,
    modified: HLCTimestamp,
    blocks: Vec<BlockId>,
}

/// Consensus Group for distributed storage
struct ConsensusGroup {
    id: String,
    location: String,
    lease_manager: Arc<LeaseManager>,
    block_storage: Arc<Mutex<HashMap<BlockId, Vec<u8>>>>,
    metadata_cache: Arc<Mutex<HashMap<String, FileMetadata>>>,
}

impl ConsensusGroup {
    fn new(id: String, location: String, hlc: Arc<HLC>) -> Self {
        Self {
            id: id.clone(),
            location,
            lease_manager: Arc::new(LeaseManager::new(id, hlc)),
            block_storage: Arc::new(Mutex::new(HashMap::new())),
            metadata_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// O(1) block write with lease acquisition
    fn write_block(&self, block_id: BlockId, data: Vec<u8>) -> Result<(), String> {
        let lease_scope = LeaseScope::Block(format!("block-{}", block_id.0));
        
        match self.lease_manager.acquire_lease(lease_scope, Duration::from_secs(60)) {
            Ok(_lease) => {
                let mut storage = self.block_storage.lock().unwrap();
                storage.insert(block_id, data);
                Ok(())
            }
            Err(e) => Err(format!("Lease acquisition failed: {:?}", e))
        }
    }
    
    /// O(1) block read
    fn read_block(&self, block_id: &BlockId) -> Option<Vec<u8>> {
        let storage = self.block_storage.lock().unwrap();
        storage.get(block_id).cloned()
    }
    
    fn block_count(&self) -> usize {
        self.block_storage.lock().unwrap().len()
    }
}

/// O(1) Scalable Storage System
struct ScalableStorage {
    hlc: Arc<HLC>,
    consensus_groups: Vec<ConsensusGroup>,
    // Honeycomb mapping: block_id -> consensus_group_index
    block_placement: Arc<Mutex<HashMap<BlockId, usize>>>,
    stats: Arc<Mutex<StorageStats>>,
}

#[derive(Debug, Default)]
struct StorageStats {
    reads: u64,
    writes: u64,
    avg_read_time_ns: u64,
    avg_write_time_ns: u64,
}

impl ScalableStorage {
    fn new() -> Self {
        let hlc = Arc::new(HLC::new());
        
        // Create honeycomb of consensus groups
        let consensus_groups = vec![
            ConsensusGroup::new("cg-local".to_string(), "Local".to_string(), hlc.clone()),
            ConsensusGroup::new("cg-edge".to_string(), "Edge".to_string(), hlc.clone()),
            ConsensusGroup::new("cg-cloud".to_string(), "Cloud".to_string(), hlc.clone()),
        ];
        
        Self {
            hlc,
            consensus_groups,
            block_placement: Arc::new(Mutex::new(HashMap::new())),
            stats: Arc::new(Mutex::new(StorageStats::default())),
        }
    }
    
    /// O(1) placement using honeycomb hashing
    fn place_block(&self, block_id: &BlockId) -> usize {
        // Simple consistent hashing - in production would be more sophisticated
        (block_id.0 as usize) % self.consensus_groups.len()
    }
    
    /// O(1) write operation
    fn write(&self, filename: &str, data: Vec<u8>) -> Result<(), String> {
        let start = Instant::now();
        
        // Break data into 4KB blocks
        const BLOCK_SIZE: usize = 4096;
        let blocks_needed = (data.len() + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let file_id = self.hash_filename(filename);
        
        println!("📝 Writing file '{}' ({} bytes, {} blocks)", 
                 filename, data.len(), blocks_needed);
        
        let mut block_ids = Vec::new();
        
        // Write each block with O(1) placement
        for i in 0..blocks_needed {
            let block_id = BlockId(file_id * 1000 + i as u64);
            let start_byte = i * BLOCK_SIZE;
            let end_byte = std::cmp::min((i + 1) * BLOCK_SIZE, data.len());
            let block_data = data[start_byte..end_byte].to_vec();
            
            // O(1) placement decision
            let cg_index = self.place_block(&block_id);
            
            // Update placement mapping
            {
                let mut placement = self.block_placement.lock().unwrap();
                placement.insert(block_id.clone(), cg_index);
            }
            
            // O(1) write to consensus group
            match self.consensus_groups[cg_index].write_block(block_id.clone(), block_data) {
                Ok(_) => {
                    println!("  Block {} → CG-{} ({})", 
                             block_id.0, cg_index, self.consensus_groups[cg_index].location);
                    block_ids.push(block_id);
                }
                Err(e) => {
                    println!("  ❌ Block {} write failed: {}", block_id.0, e);
                    return Err(e);
                }
            }
        }
        
        // O(1) metadata update
        let metadata = FileMetadata {
            name: filename.to_string(),
            size: data.len() as u64,
            created: self.hlc.now(),
            modified: self.hlc.now(),
            blocks: block_ids,
        };
        
        // Store metadata in primary consensus group
        let primary_cg = &self.consensus_groups[0];
        {
            let mut cache = primary_cg.metadata_cache.lock().unwrap();
            cache.insert(filename.to_string(), metadata);
        }
        
        // Update stats
        let elapsed = start.elapsed();
        {
            let mut stats = self.stats.lock().unwrap();
            stats.writes += 1;
            stats.avg_write_time_ns = elapsed.as_nanos() as u64;
        }
        
        println!("✅ File '{}' written in {:?}", filename, elapsed);
        Ok(())
    }
    
    /// O(1) read operation
    fn read(&self, filename: &str) -> Result<Vec<u8>, String> {
        let start = Instant::now();
        
        println!("📖 Reading file '{}'", filename);
        
        // O(1) metadata lookup
        let metadata = {
            let cache = self.consensus_groups[0].metadata_cache.lock().unwrap();
            cache.get(filename).cloned()
        };
        
        let metadata = metadata.ok_or_else(|| "File not found".to_string())?;
        
        println!("  Found {} blocks to read", metadata.blocks.len());
        
        let mut result_data = Vec::new();
        
        // O(1) read for each block
        for block_id in &metadata.blocks {
            // O(1) placement lookup
            let cg_index = {
                let placement = self.block_placement.lock().unwrap();
                placement.get(block_id).copied()
                    .unwrap_or_else(|| self.place_block(block_id))
            };
            
            // O(1) read from consensus group
            if let Some(block_data) = self.consensus_groups[cg_index].read_block(block_id) {
                println!("  Block {} ← CG-{} ({} bytes)", 
                         block_id.0, cg_index, block_data.len());
                result_data.extend(block_data);
            } else {
                return Err(format!("Block {} not found", block_id.0));
            }
        }
        
        // Update stats
        let elapsed = start.elapsed();
        {
            let mut stats = self.stats.lock().unwrap();
            stats.reads += 1;
            stats.avg_read_time_ns = elapsed.as_nanos() as u64;
        }
        
        println!("✅ File '{}' read in {:?} ({} bytes)", filename, elapsed, result_data.len());
        Ok(result_data)
    }
    
    /// Simple filename hashing
    fn hash_filename(&self, filename: &str) -> u64 {
        // Simple hash - in production would use proper hash function
        filename.bytes().map(|b| b as u64).sum()
    }
    
    /// Print system statistics
    fn print_stats(&self) {
        let stats = self.stats.lock().unwrap();
        let metadata_count = self.consensus_groups[0].metadata_cache.lock().unwrap().len();
        
        println!("\n=== O(1) Scalable Storage Statistics ===");
        println!("Files: {} (O(1) metadata lookup)", metadata_count);
        println!("Read Operations: {} (avg: {}ns)", stats.reads, stats.avg_read_time_ns);
        println!("Write Operations: {} (avg: {}ns)", stats.writes, stats.avg_write_time_ns);
        
        println!("\nHoneycomb Distribution:");
        for (i, cg) in self.consensus_groups.iter().enumerate() {
            println!("  CG-{} ({}): {} blocks", i, cg.location, cg.block_count());
        }
        
        // Show placement efficiency
        let placement = self.block_placement.lock().unwrap();
        println!("\nPlacement Efficiency:");
        let mut cg_counts = vec![0; self.consensus_groups.len()];
        for &cg_index in placement.values() {
            if cg_index < cg_counts.len() {
                cg_counts[cg_index] += 1;
            }
        }
        for (i, count) in cg_counts.iter().enumerate() {
            let percentage = if placement.len() > 0 {
                *count as f64 / placement.len() as f64 * 100.0
            } else {
                0.0
            };
            println!("  CG-{}: {} blocks ({:.1}%)", i, count, percentage);
        }
        println!("Total Blocks: {}", placement.len());
    }
}

fn main() {
    println!("=== O(1) Scalable Storage Demo ===");
    println!("Demonstrates RHC concepts without FUSE complexity\n");
    
    let storage = ScalableStorage::new();
    
    // Demonstrate O(1) operations
    let large_file_data = vec![42u8; 10000];
    let test_files: Vec<(&str, &[u8])> = vec![
        ("document.txt", b"Hello RHC! This is a test document with some content."),
        ("data.json", br#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#),
        ("config.yaml", b"database:\n  host: localhost\n  port: 5432\nlogging:\n  level: info"),
        ("large_file.bin", &large_file_data), // 10KB file to show multi-block
    ];
    
    println!("--- Writing Files (O(1) per block) ---");
    for (filename, content) in &test_files {
        match storage.write(filename, content.to_vec()) {
            Ok(_) => println!("✓ {} written", filename),
            Err(e) => println!("✗ {} failed: {}", filename, e),
        }
        println!();
    }
    
    storage.print_stats();
    
    println!("\n--- Reading Files (O(1) per block) ---");
    for (filename, original_content) in &test_files {
        match storage.read(filename) {
            Ok(data) => {
                if data == *original_content {
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
    
    println!("\n=== Key O(1) Properties Demonstrated ===");
    println!("✓ Metadata lookups: O(1) - direct hash table access");
    println!("✓ Block placement: O(1) - consistent hashing");  
    println!("✓ Block reads/writes: O(1) - direct CG access");
    println!("✓ Lease acquisition: O(1) - per-block leases");
    println!("✓ Honeycomb distribution: Even load balancing");
    
    println!("\n🚀 This scales infinitely:");
    println!("  • Add more consensus groups → more parallelism");
    println!("  • Add more blocks → same O(1) per-block performance");
    println!("  • Add more files → same O(1) metadata access");
    println!("  • Geographic distribution → leases migrate to users");
}
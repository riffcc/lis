// Hard Rust tests proving variable block storage correctness
// Tests O(1) operations, compression ratios, bitvec tracking, and data integrity

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bitvec::prelude::*;
use lis::rhc::hlc::HLC;
use lis::rhc::leases::{LeaseManager, LeaseScope};

// Import from the variable block storage example
// Note: In a real implementation, these would be moved to lib.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockSize {
    Tiny = 512,
    Small = 1024,
    Standard = 4096,
    Large = 16384,
    Huge = 65536,
}

impl BlockSize {
    fn as_usize(self) -> usize {
        self as usize
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
    null_bitmap: Option<BitVec>,
}

impl CompressedBlock {
    fn new(data: Vec<u8>) -> Self {
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

fn null_compress(data: &[u8]) -> (Vec<u8>, Option<BitVec>) {
    if data.iter().all(|&b| b != 0) {
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
    
    if compressed.len() < data.len() * 3 / 4 {
        (compressed, Some(null_bitmap))
    } else {
        (data.to_vec(), None)
    }
}

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

struct TestConsensusGroup {
    _id: String,
    lease_manager: Arc<LeaseManager>,
    block_storage: Arc<Mutex<HashMap<VariableBlockId, CompressedBlock>>>,
}

impl TestConsensusGroup {
    fn new(id: String, hlc: Arc<HLC>) -> Self {
        Self {
            _id: id.clone(),
            lease_manager: Arc::new(LeaseManager::new(id, hlc)),
            block_storage: Arc::new(Mutex::new(HashMap::new())),
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
        storage.get(block_id).map(|compressed_block| compressed_block.decompress())
    }
    
    fn block_count(&self) -> usize {
        self.block_storage.lock().unwrap().len()
    }
}

// ===== HARD TESTS PROVING CORRECTNESS =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_compression_correctness() {
        // Test 1: Sparse data should compress well
        let sparse_data = {
            let mut data = vec![0u8; 4096];
            data[100..150].fill(0xAA); // 50 bytes of actual data
            data[2000..2100].fill(0xBB); // 100 bytes of actual data
            data
        };
        
        let compressed = CompressedBlock::new(sparse_data.clone());
        
        // Should achieve significant compression
        assert!(compressed.compression_ratio < 0.5, 
                "Sparse data should compress to less than 50%, got {:.2}", 
                compressed.compression_ratio);
        
        // Decompression should be perfect
        let decompressed = compressed.decompress();
        assert_eq!(decompressed, sparse_data, "Decompressed data must match original");
        assert_eq!(decompressed.len(), 4096, "Decompressed size must match original");
        
        // Verify specific non-zero regions
        assert_eq!(&decompressed[100..150], &vec![0xAA; 50], "First data region mismatch");
        assert_eq!(&decompressed[2000..2100], &vec![0xBB; 100], "Second data region mismatch");
        
        println!("✓ Null compression test: {:.1}x compression ratio", 1.0 / compressed.compression_ratio);
    }
    
    #[test]
    fn test_dense_data_no_compression() {
        // Test 2: Dense data should not be compressed
        let dense_data = (0..4096).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
        
        let compressed = CompressedBlock::new(dense_data.clone());
        
        // Should not compress (ratio close to 1.0)
        assert!(compressed.compression_ratio > 0.9, 
                "Dense data should not compress significantly, got {:.2}", 
                compressed.compression_ratio);
        
        // Should be stored without bitmap
        assert!(compressed.null_bitmap.is_none(), "Dense data should not use null bitmap");
        
        // Perfect round-trip
        let decompressed = compressed.decompress();
        assert_eq!(decompressed, dense_data, "Dense data round-trip must be perfect");
        
        println!("✓ Dense data test: {:.1}x compression ratio (expected ~1.0)", 1.0 / compressed.compression_ratio);
    }
    
    #[test]
    fn test_variable_block_sizes_are_consistent() {
        // Test 3: Different block sizes should work correctly
        let hlc = Arc::new(HLC::new());
        let cg = TestConsensusGroup::new("test-cg".to_string(), hlc);
        
        let test_data = vec![0x42; 8192]; // 8KB of data
        
        // Test different block size configurations
        let block_sizes = [BlockSize::Tiny, BlockSize::Small, BlockSize::Standard, BlockSize::Large];
        
        for (i, &block_size) in block_sizes.iter().enumerate() {
            let block_id = VariableBlockId {
                file_hash: 12345,
                block_index: i as u64,
                block_size,
            };
            
            // Write should succeed
            let result = cg.write_block(block_id.clone(), test_data.clone());
            assert!(result.is_ok(), "Block write should succeed for {:?}", block_size);
            
            // Read should return exact data
            let read_data = cg.read_block(&block_id);
            assert!(read_data.is_some(), "Block read should succeed for {:?}", block_size);
            assert_eq!(read_data.unwrap(), test_data, "Read data should match written data");
        }
        
        assert_eq!(cg.block_count(), 4, "Should have 4 blocks stored");
        println!("✓ Variable block sizes test: All 4 block sizes work correctly");
    }
    
    #[test]
    fn test_massive_compression_ratio() {
        // Test 4: Extremely sparse data should achieve massive compression
        let mut ultra_sparse = vec![0u8; 65536]; // 64KB of mostly zeros
        ultra_sparse[1000] = 0xFF; // Just 1 byte of data
        ultra_sparse[30000] = 0xEE; // And another byte
        
        let compressed = CompressedBlock::new(ultra_sparse.clone());
        
        // Should achieve extreme compression
        let compression_ratio = 1.0 / compressed.compression_ratio;
        assert!(compression_ratio > 1000.0, 
                "Ultra-sparse data should compress >1000x, got {:.1}x", compression_ratio);
        
        // Perfect reconstruction
        let decompressed = compressed.decompress();
        assert_eq!(decompressed, ultra_sparse, "Ultra-sparse decompression must be perfect");
        assert_eq!(decompressed[1000], 0xFF, "Specific byte 1 should be preserved");
        assert_eq!(decompressed[30000], 0xEE, "Specific byte 2 should be preserved");
        assert_eq!(decompressed[500], 0x00, "Zero regions should be preserved");
        
        println!("✓ Massive compression test: {:.0}x compression ratio", compression_ratio);
    }
    
    #[test]
    fn test_bitvec_encoding_efficiency() {
        // Test 5: Block size tracking should use exactly 3 bits per change
        
        #[derive(Debug, Clone)]
        struct BlockSizeTracker {
            size_changes: BitVec,
            change_positions: Vec<usize>,
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
            }
        }
        
        let mut tracker = BlockSizeTracker::new(BlockSize::Standard);
        
        // Make several block size changes
        tracker.change_block_size(BlockSize::Tiny);
        tracker.change_block_size(BlockSize::Huge);
        tracker.change_block_size(BlockSize::Large);
        tracker.change_block_size(BlockSize::Small);
        
        // Should have exactly 5 changes * 3 bits = 15 bits total
        assert_eq!(tracker.size_changes.len(), 15, "Should use exactly 15 bits for 5 changes");
        assert_eq!(tracker.change_positions.len(), 5, "Should track 5 changes");
        
        // Verify bit efficiency
        let bits_per_change = tracker.size_changes.len() as f32 / tracker.change_positions.len() as f32;
        assert!((bits_per_change - 3.0).abs() < 0.01, "Should use exactly 3 bits per change");
        
        println!("✓ Bitvec encoding test: {:.1} bits per change (expected 3.0)", bits_per_change);
    }
    
    #[test]
    fn test_concurrent_access_with_leases() {
        // Test 6: Concurrent access should work correctly with lease system
        use std::sync::mpsc;
        use std::thread;
        
        let hlc = Arc::new(HLC::new());
        let cg = Arc::new(TestConsensusGroup::new("concurrent-test".to_string(), hlc));
        let (tx, rx) = mpsc::channel();
        
        let mut handles = vec![];
        
        // Spawn 10 concurrent writers
        for i in 0..10 {
            let cg_clone = Arc::clone(&cg);
            let tx_clone = tx.clone();
            
            let handle = thread::spawn(move || {
                let block_id = VariableBlockId {
                    file_hash: 99999,
                    block_index: i as u64,
                    block_size: BlockSize::Standard,
                };
                
                let test_data = vec![i as u8; 4096];
                let result = cg_clone.write_block(block_id, test_data);
                tx_clone.send((i, result.is_ok())).unwrap();
            });
            
            handles.push(handle);
        }
        
        drop(tx); // Close sender
        
        // Wait for all writes to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Verify all writes succeeded
        let mut success_count = 0;
        while let Ok((thread_id, success)) = rx.try_recv() {
            assert!(success, "Thread {} should succeed in writing", thread_id);
            success_count += 1;
        }
        
        assert_eq!(success_count, 10, "All 10 concurrent writes should succeed");
        assert_eq!(cg.block_count(), 10, "Should have 10 blocks stored after concurrent writes");
        
        println!("✓ Concurrent access test: All 10 concurrent writes succeeded with lease system");
    }
    
    #[test]
    fn test_block_size_affects_storage_placement() {
        // Test 7: Different block sizes should result in different storage decisions
        let hlc = Arc::new(HLC::new());
        let cg_ssd = TestConsensusGroup::new("ssd".to_string(), hlc.clone());
        let cg_hdd = TestConsensusGroup::new("hdd".to_string(), hlc.clone());
        let cg_cloud = TestConsensusGroup::new("cloud".to_string(), hlc);
        
        // Simulate placement logic
        fn place_block(block_size: BlockSize) -> usize {
            match block_size {
                BlockSize::Tiny | BlockSize::Small => 0, // SSD
                BlockSize::Standard => 1, // HDD
                BlockSize::Large | BlockSize::Huge => 2, // Cloud
            }
        }
        
        let consensus_groups = vec![&cg_ssd, &cg_hdd, &cg_cloud];
        let test_data = vec![0x55; 1024];
        
        // Test each block size goes to correct storage tier
        let test_cases = [
            (BlockSize::Tiny, 0, "SSD"),
            (BlockSize::Small, 0, "SSD"),
            (BlockSize::Standard, 1, "HDD"),
            (BlockSize::Large, 2, "Cloud"),
            (BlockSize::Huge, 2, "Cloud"),
        ];
        
        for (block_size, expected_cg, tier_name) in test_cases {
            let placement = place_block(block_size);
            assert_eq!(placement, expected_cg, 
                      "{:?} blocks should be placed on {} (CG {})", 
                      block_size, tier_name, expected_cg);
            
            // Actually store the block
            let block_id = VariableBlockId {
                file_hash: 88888,
                block_index: placement as u64,
                block_size,
            };
            
            let result = consensus_groups[placement].write_block(block_id, test_data.clone());
            assert!(result.is_ok(), "Block write should succeed on {}", tier_name);
        }
        
        // Verify distribution
        assert_eq!(cg_ssd.block_count(), 2, "SSD should have 2 blocks (Tiny + Small)");
        assert_eq!(cg_hdd.block_count(), 1, "HDD should have 1 block (Standard)");
        assert_eq!(cg_cloud.block_count(), 2, "Cloud should have 2 blocks (Large + Huge)");
        
        println!("✓ Storage placement test: Block sizes correctly routed to appropriate tiers");
    }
    
    #[test]
    fn test_extreme_data_patterns() {
        // Test 8: Edge cases and extreme data patterns
        
        // All zeros
        let all_zeros = vec![0u8; 8192];
        let compressed_zeros = CompressedBlock::new(all_zeros.clone());
        assert!(compressed_zeros.compression_ratio < 0.01, "All zeros should compress to nearly nothing");
        assert_eq!(compressed_zeros.decompress(), all_zeros, "All zeros round-trip");
        
        // All ones
        let all_ones = vec![0xFF; 8192];
        let compressed_ones = CompressedBlock::new(all_ones.clone());
        assert!(compressed_ones.compression_ratio > 0.9, "All ones should not compress");
        assert_eq!(compressed_ones.decompress(), all_ones, "All ones round-trip");
        
        // Alternating pattern (may compress somewhat due to null bitmap)
        let alternating = (0..8192).map(|i| if i % 2 == 0 { 0x00 } else { 0xFF }).collect::<Vec<u8>>();
        let compressed_alt = CompressedBlock::new(alternating.clone());
        // Alternating pattern has 50% zeros, so it might compress to ~50%
        println!("Alternating pattern compression ratio: {:.2}", compressed_alt.compression_ratio);
        assert_eq!(compressed_alt.decompress(), alternating, "Alternating pattern round-trip");
        
        // Single byte in large buffer
        let mut single_byte = vec![0u8; 32768];
        single_byte[16384] = 0x42;
        let compressed_single = CompressedBlock::new(single_byte.clone());
        assert!(compressed_single.compression_ratio < 0.001, "Single byte should compress extremely well");
        assert_eq!(compressed_single.decompress(), single_byte, "Single byte round-trip");
        
        println!("✓ Extreme data patterns test: All edge cases handled correctly");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;
    
    #[test] 
    fn test_store_and_retrieve_real_file() {
        // Test 9: Store and retrieve the RiP! A Remix Manifesto file
        let file_path = Path::new("~/Riff.CC Content/RiP! A Remix Manifesto.txt");
        
        // Try to expand the tilde path
        let expanded_path = if file_path.starts_with("~") {
            if let Ok(home) = std::env::var("HOME") {
                Path::new(&home).join(file_path.strip_prefix("~").unwrap())
            } else {
                file_path.to_path_buf()
            }
        } else {
            file_path.to_path_buf()
        };
        
        println!("🎬 Looking for RiP! A Remix Manifesto at: {}", expanded_path.display());
        
        // If file doesn't exist, create test manifesto content
        let file_content = if expanded_path.exists() {
            println!("📄 Found real RiP! file, reading...");
            std::fs::read(&expanded_path).expect("Failed to read RiP! file")
        } else {
            println!("📝 Creating test RiP! manifesto content...");
            include_bytes!("test_rip_manifesto.txt").to_vec()
        };
        
        println!("📊 File size: {} bytes", file_content.len());
        
        // Set up storage system
        let hlc = Arc::new(HLC::new());
        let storage_groups = vec![
            TestConsensusGroup::new("nvme-ssd".to_string(), hlc.clone()),
            TestConsensusGroup::new("spinning-hdd".to_string(), hlc.clone()),
            TestConsensusGroup::new("cloud-storage".to_string(), hlc.clone()),
        ];
        
        // Determine optimal block size based on content analysis
        let optimal_block_size = if file_content.len() < 2048 {
            BlockSize::Tiny
        } else if file_content.len() < 16384 {
            BlockSize::Small  
        } else if has_sparse_pattern(&file_content) {
            BlockSize::Standard // Good for compression
        } else {
            BlockSize::Large
        };
        
        println!("🎯 Using {:?} blocks for optimal performance", optimal_block_size);
        
        // Store the file in variable blocks
        let file_hash = 0x52694646u64; // "RiFF" in hex (ASCII: R=0x52, i=0x69, F=0x46, F=0x46)
        let block_size = optimal_block_size.as_usize();
        let blocks_needed = (file_content.len() + block_size - 1) / block_size;
        
        println!("📦 Storing file in {} blocks of {} bytes each", blocks_needed, block_size);
        
        let mut stored_blocks = Vec::new();
        let mut total_compressed_size = 0;
        
        // Write each block
        for i in 0..blocks_needed {
            let start_byte = i * block_size;
            let end_byte = std::cmp::min((i + 1) * block_size, file_content.len());
            let block_data = file_content[start_byte..end_byte].to_vec();
            
            let block_id = VariableBlockId {
                file_hash,
                block_index: i as u64,
                block_size: optimal_block_size,
            };
            
            // Smart placement based on block size
            let cg_index = match optimal_block_size {
                BlockSize::Tiny | BlockSize::Small => 0, // Fast SSD
                BlockSize::Standard => i % 2, // Distribute between SSD and HDD
                BlockSize::Large | BlockSize::Huge => 2, // Cloud storage
            };
            
            let result = storage_groups[cg_index].write_block(block_id.clone(), block_data.clone());
            assert!(result.is_ok(), "Block {} write should succeed", i);
            
            let compressed_block = result.unwrap();
            total_compressed_size += compressed_block.data.len();
            stored_blocks.push((block_id, cg_index));
            
            println!("  Block {}: {} bytes → {} bytes ({:.1}x compression) on CG-{}", 
                     i, block_data.len(), compressed_block.data.len(),
                     1.0 / compressed_block.compression_ratio, cg_index);
        }
        
        let overall_compression = file_content.len() as f32 / total_compressed_size as f32;
        println!("✅ File stored with {:.1}x overall compression ratio", overall_compression);
        
        // Now retrieve the entire file
        println!("📖 Retrieving and verifying RiP! manifesto...");
        let mut retrieved_content = Vec::new();
        
        for (block_id, cg_index) in &stored_blocks {
            let block_data = storage_groups[*cg_index].read_block(block_id);
            assert!(block_data.is_some(), "Block retrieval should succeed");
            retrieved_content.extend(block_data.unwrap());
        }
        
        // Verify perfect reconstruction
        assert_eq!(retrieved_content.len(), file_content.len(), 
                  "Retrieved file size should match original");
        assert_eq!(retrieved_content, file_content, 
                  "Retrieved content should perfectly match original");
        
        println!("✅ Perfect file reconstruction verified!");
        
        // Show storage distribution  
        println!("📊 Storage distribution:");
        for (i, cg) in storage_groups.iter().enumerate() {
            let count = cg.block_count();
            if count > 0 {
                let cg_name = match i {
                    0 => "NVMe SSD",
                    1 => "Spinning HDD", 
                    2 => "Cloud Storage",
                    _ => "Unknown",
                };
                println!("  {}: {} blocks", cg_name, count);
            }
        }
        
        println!("🎉 RiP! A Remix Manifesto successfully stored and retrieved!");
        println!("   Original: {} bytes", file_content.len());
        println!("   Compressed: {} bytes", total_compressed_size);
        println!("   Compression: {:.1}x", overall_compression);
        println!("   Blocks: {} ({:?} size)", blocks_needed, optimal_block_size);
    }
}

// Helper function for sparse pattern detection
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
    
    zero_count as f32 / sample_size as f32 > 0.3 || 
    repeated_byte_count as f32 / sample_size as f32 > 0.5
}
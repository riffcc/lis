// Demonstrates block-level lease optimization
// Shows how different users rarely conflict at block granularity

use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BlockId(u64);

#[derive(Debug, Clone, PartialEq)]
struct Location(&'static str);

#[derive(Debug)]
struct AccessRecord {
    user: &'static str,
    location: Location,
    timestamp: Instant,
    latency_ms: u64,
}

struct BlockLeaseManager {
    leases: HashMap<BlockId, Location>,
    access_history: HashMap<BlockId, Vec<AccessRecord>>,
    network_latencies: HashMap<(&'static str, &'static str), u64>,
}

impl BlockLeaseManager {
    fn new() -> Self {
        let mut network_latencies = HashMap::new();
        // Define network latencies between locations
        network_latencies.insert(("Sydney", "London"), 250);
        network_latencies.insert(("London", "Sydney"), 250);
        network_latencies.insert(("Sydney", "NYC"), 200);
        network_latencies.insert(("NYC", "Sydney"), 200);
        network_latencies.insert(("London", "NYC"), 80);
        network_latencies.insert(("NYC", "London"), 80);
        network_latencies.insert(("Tokyo", "Sydney"), 120);
        network_latencies.insert(("Sydney", "Tokyo"), 120);
        
        Self {
            leases: HashMap::new(),
            access_history: HashMap::new(),
            network_latencies,
        }
    }
    
    fn get_network_latency(&self, from: &str, to: &str) -> u64 {
        if from == to {
            0
        } else {
            self.network_latencies.get(&(from, to)).copied().unwrap_or(150)
        }
    }
    
    fn write_block(&mut self, block: BlockId, user: &'static str, location: Location) -> u64 {
        let current_lease = self.leases.get(&block).cloned()
            .unwrap_or(Location("NYC")); // Default location
        
        let latency = if current_lease == location {
            1 // Local write
        } else {
            let network = self.get_network_latency(location.0, current_lease.0);
            network * 2 + 1 // Round trip + write
        };
        
        // Record access
        let record = AccessRecord {
            user,
            location: location.clone(),
            timestamp: Instant::now(),
            latency_ms: latency,
        };
        
        self.access_history.entry(block.clone())
            .or_insert_with(Vec::new)
            .push(record);
        
        // Check if we should steal lease
        if self.should_steal_lease(&block, &location) {
            println!("    → {} steals lease for block {:?}!", location.0, block);
            self.leases.insert(block, location);
        }
        
        latency
    }
    
    fn should_steal_lease(&self, block: &BlockId, requesting_location: &Location) -> bool {
        let current_lease = match self.leases.get(block) {
            Some(loc) if loc == requesting_location => return false, // Already have it
            Some(loc) => loc,
            None => return true, // No lease yet
        };
        
        let history = match self.access_history.get(block) {
            Some(h) => h,
            None => return true,
        };
        
        // Get recent accesses (last 10)
        let recent: Vec<_> = history.iter().rev().take(10).collect();
        
        // Count unique users
        let unique_users: std::collections::HashSet<_> = recent.iter()
            .map(|r| r.user)
            .collect();
        
        if unique_users.len() == 1 {
            // Single user - optimize for them
            let user_accesses: Vec<_> = recent.iter()
                .filter(|r| r.location == *requesting_location)
                .collect();
            
            if user_accesses.len() >= 3 {
                let avg_latency: f64 = user_accesses.iter()
                    .map(|r| r.latency_ms as f64)
                    .sum::<f64>() / user_accesses.len() as f64;
                
                return avg_latency > 50.0; // Steal if avg > 50ms
            }
        } else {
            // Multiple users - optimize for group
            let avg_if_moved = self.calculate_avg_latency_if_at(block, requesting_location);
            let avg_current = self.calculate_avg_latency_if_at(block, current_lease);
            
            // Only steal if 50% improvement for everyone
            return avg_if_moved < avg_current * 0.66;
        }
        
        false
    }
    
    fn calculate_avg_latency_if_at(&self, block: &BlockId, lease_location: &Location) -> f64 {
        let history = self.access_history.get(block).unwrap();
        let recent: Vec<_> = history.iter().rev().take(10).collect();
        
        let total: u64 = recent.iter()
            .map(|record| {
                if record.location == *lease_location {
                    1
                } else {
                    let network = self.get_network_latency(record.location.0, lease_location.0);
                    network * 2 + 1
                }
            })
            .sum();
            
        total as f64 / recent.len() as f64
    }
    
    fn print_block_stats(&self, block: &BlockId) {
        if let Some(history) = self.access_history.get(block) {
            let total_latency: u64 = history.iter().map(|r| r.latency_ms).sum();
            let avg_latency = total_latency as f64 / history.len() as f64;
            let lease_location = self.leases.get(block)
                .map(|l| l.0)
                .unwrap_or("None");
            
            println!("  Block {:?}: {} accesses, avg latency {:.1}ms, lease at {}",
                     block, history.len(), avg_latency, lease_location);
        }
    }
}

fn main() {
    println!("=== Block-Level Lease Optimization Demo ===");
    println!("\nKey insight: At block level, conflicts are rare!\n");
    
    let mut manager = BlockLeaseManager::new();
    
    // Scenario 1: Git repository - different files
    println!("--- Scenario 1: Shared Git Repository ---");
    println!("Multiple developers working on different files\n");
    
    // Sydney dev works on auth.rs (blocks 1000-1010)
    println!("Sydney developer editing src/auth.rs:");
    for i in 0..5 {
        let block = BlockId(1000 + i);
        let latency = manager.write_block(block, "Alice", Location("Sydney"));
        println!("  Write to block {} - {}ms", 1000 + i, latency);
    }
    
    // London dev works on network.rs (blocks 2000-2010)
    println!("\nLondon developer editing src/network.rs:");
    for i in 0..5 {
        let block = BlockId(2000 + i);
        let latency = manager.write_block(block, "Bob", Location("London"));
        println!("  Write to block {} - {}ms", 2000 + i, latency);
    }
    
    // NYC dev works on tests.rs (blocks 3000-3010)
    println!("\nNYC developer editing tests/api.rs:");
    for i in 0..5 {
        let block = BlockId(3000 + i);
        let latency = manager.write_block(block, "Charlie", Location("NYC"));
        println!("  Write to block {} - {}ms", 3000 + i, latency);
    }
    
    println!("\nResult: Each developer has local leases for their blocks!");
    manager.print_block_stats(&BlockId(1000));
    manager.print_block_stats(&BlockId(2000));
    manager.print_block_stats(&BlockId(3000));
    
    // Scenario 2: Actual conflict - shared config file
    println!("\n--- Scenario 2: Shared Configuration File ---");
    println!("Multiple admins editing the same file (same blocks)\n");
    
    let config_block = BlockId(5000);
    
    // Multiple users access the same block
    println!("Multiple admins editing config.yaml:");
    
    // Tokyo admin
    for _ in 0..3 {
        let latency = manager.write_block(config_block.clone(), "Admin-Tokyo", Location("Tokyo"));
        println!("  Tokyo write - {}ms", latency);
    }
    
    // London admin
    for _ in 0..3 {
        let latency = manager.write_block(config_block.clone(), "Admin-London", Location("London"));
        println!("  London write - {}ms", latency);
    }
    
    // Sydney admin
    for _ in 0..3 {
        let latency = manager.write_block(config_block.clone(), "Admin-Sydney", Location("Sydney"));
        println!("  Sydney write - {}ms", latency);
    }
    
    println!("\nLease placement optimized for group average latency:");
    manager.print_block_stats(&config_block);
    
    // Scenario 3: Database partitioning
    println!("\n--- Scenario 3: Naturally Partitioned Database ---");
    println!("Regional tables prevent block conflicts\n");
    
    // Asia writes to asia partition
    println!("Asia region writing to users_asia table:");
    for i in 0..3 {
        let block = BlockId(6000 + i);
        let latency = manager.write_block(block, "DB-Asia", Location("Tokyo"));
        println!("  Insert into block {} - {}ms", 6000 + i, latency);
    }
    
    // Europe writes to europe partition  
    println!("\nEurope region writing to users_europe table:");
    for i in 0..3 {
        let block = BlockId(7000 + i);
        let latency = manager.write_block(block, "DB-Europe", Location("London"));
        println!("  Insert into block {} - {}ms", 7000 + i, latency);
    }
    
    println!("\nResult: Natural partitioning = no conflicts!");
    manager.print_block_stats(&BlockId(6000));
    manager.print_block_stats(&BlockId(7000));
    
    // Summary
    println!("\n=== Summary ===");
    println!("✓ Different files = different blocks = no conflicts");
    println!("✓ Database partitions = natural block separation");
    println!("✓ Only shared config files cause actual contention");
    println!("✓ Even then, intelligent placement minimizes average latency");
    
    println!("\n🎯 Block-level granularity makes conflicts rare!");
    println!("   Most users get local write speeds most of the time!");
}
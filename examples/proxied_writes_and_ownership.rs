// Demonstrates the Lis philosophy:
// - With lease: "you own this folder and here, you are king" (burst buffer speeds)
// - Without lease: "slow write, fast reads" (proxied writes through lease holder)

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct WriteRequest {
    id: u64,
    data: String,
    size_bytes: usize,
    origin: String,
}

#[derive(Debug)]
struct ProxyStats {
    local_writes: u64,
    proxied_writes: u64,
    proxy_latency_ms: Vec<u64>,
}

struct DataNode {
    id: String,
    location: String,
    has_lease: Arc<Mutex<bool>>,
    stats: Arc<Mutex<ProxyStats>>,
    write_queue: Arc<Mutex<Vec<WriteRequest>>>,
}

impl DataNode {
    fn new(id: String, location: String) -> Self {
        Self {
            id,
            location,
            has_lease: Arc::new(Mutex::new(false)),
            stats: Arc::new(Mutex::new(ProxyStats {
                local_writes: 0,
                proxied_writes: 0,
                proxy_latency_ms: Vec::new(),
            })),
            write_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Process a write request - either locally or proxy to lease holder
    fn write(&self, request: WriteRequest, lease_holder: Option<&DataNode>) -> Duration {
        let start = Instant::now();
        
        if *self.has_lease.lock().unwrap() {
            // WE ARE KING! Burst buffer speeds!
            std::thread::sleep(Duration::from_micros(100)); // SSD latency
            self.stats.lock().unwrap().local_writes += 1;
            println!("  {} (LEASE HOLDER): Fast local write #{} (100μs)", 
                     self.id, request.id);
        } else if let Some(holder) = lease_holder {
            // Proxy write to lease holder
            let network_latency = self.calculate_latency(&holder.location);
            std::thread::sleep(network_latency);
            
            // Lease holder processes the write
            holder.write_queue.lock().unwrap().push(request.clone());
            std::thread::sleep(Duration::from_micros(100)); // Remote SSD
            
            // Acknowledgment back
            std::thread::sleep(network_latency);
            
            let total_ms = (network_latency.as_millis() * 2 + Duration::from_micros(100).as_millis()) as u64;
            let mut stats = self.stats.lock().unwrap();
            stats.proxied_writes += 1;
            stats.proxy_latency_ms.push(total_ms);
            
            println!("  {} -> {} (PROXIED): Write #{} via lease holder ({}ms total)",
                     self.id, holder.id, request.id, total_ms);
        } else {
            println!("  {} ERROR: No lease holder available!", self.id);
        }
        
        start.elapsed()
    }

    /// Calculate network latency based on locations
    fn calculate_latency(&self, other_location: &str) -> Duration {
        match (self.location.as_str(), other_location) {
            ("Perth", "London") | ("London", "Perth") => Duration::from_millis(150),
            ("Tokyo", "London") | ("London", "Tokyo") => Duration::from_millis(120),
            ("Tokyo", "Perth") | ("Perth", "Tokyo") => Duration::from_millis(80),
            ("NYC", "London") | ("London", "NYC") => Duration::from_millis(40),
            _ => Duration::from_millis(10), // Same region
        }
    }

    fn print_stats(&self) {
        let stats = self.stats.lock().unwrap();
        println!("\n{} Stats:", self.id);
        println!("  Local writes: {} (burst buffer speed)", stats.local_writes);
        println!("  Proxied writes: {}", stats.proxied_writes);
        
        if !stats.proxy_latency_ms.is_empty() {
            let avg_latency: f64 = stats.proxy_latency_ms.iter()
                .map(|&x| x as f64)
                .sum::<f64>() / stats.proxy_latency_ms.len() as f64;
            println!("  Average proxy latency: {:.1}ms", avg_latency);
        }
    }
}

fn simulate_workload(nodes: &[DataNode], lease_holder_id: &str, duration_secs: u64) {
    println!("\n--- Simulating {} second workload ---", duration_secs);
    println!("Lease holder: {}", lease_holder_id);
    
    let lease_holder = nodes.iter().find(|n| n.id == lease_holder_id);
    let mut write_id = 1;
    
    // Simulate writes from different locations
    let workload_distribution = vec![
        ("Perth", 40),   // 40% of writes
        ("London", 30),  // 30% of writes  
        ("Tokyo", 20),   // 20% of writes
        ("NYC", 10),     // 10% of writes
    ];
    
    for (location, percentage) in workload_distribution {
        let node = nodes.iter().find(|n| n.location == location).unwrap();
        let write_count = (percentage * duration_secs / 10) as u32; // Simplified
        
        println!("\n{} attempting {} writes:", location, write_count);
        
        for _ in 0..write_count {
            let request = WriteRequest {
                id: write_id,
                data: format!("data-{}", write_id),
                size_bytes: 4096,
                origin: location.to_string(),
            };
            write_id += 1;
            
            node.write(request, lease_holder);
        }
    }
}

fn main() {
    println!("=== Lis Proxied Writes & Ownership Demo ===");
    println!("\nKey Concepts:");
    println!("1. Lease holder has burst buffer speeds (microseconds)");
    println!("2. Non-holders can still write via proxying (milliseconds)");
    println!("3. Reads are always fast from any location");
    
    // Create global data nodes
    let nodes = vec![
        DataNode::new("perth-01".to_string(), "Perth".to_string()),
        DataNode::new("london-01".to_string(), "London".to_string()),
        DataNode::new("tokyo-01".to_string(), "Tokyo".to_string()),
        DataNode::new("nyc-01".to_string(), "NYC".to_string()),
    ];
    
    // Scenario 1: Perth has the lease
    println!("\n=== Scenario 1: Perth owns the data ===");
    nodes[0].has_lease.lock().unwrap().clone_from(&true);
    simulate_workload(&nodes, "perth-01", 2);
    
    for node in &nodes {
        node.print_stats();
    }
    
    // Reset stats
    for node in &nodes {
        *node.stats.lock().unwrap() = ProxyStats {
            local_writes: 0,
            proxied_writes: 0,
            proxy_latency_ms: Vec::new(),
        };
    }
    
    // Scenario 2: Lease migrates to London
    println!("\n\n=== Scenario 2: Lease migrates to London ===");
    println!("(Active workload shifts to European business hours)");
    *nodes[0].has_lease.lock().unwrap() = false;
    *nodes[1].has_lease.lock().unwrap() = true;
    
    simulate_workload(&nodes, "london-01", 2);
    
    for node in &nodes {
        node.print_stats();
    }
    
    // Performance comparison
    println!("\n=== Performance Analysis ===");
    println!("Local writes (with lease):");
    println!("  - Latency: 100 microseconds");
    println!("  - Throughput: ~10,000 writes/second");
    println!("\nProxied writes (without lease):");
    println!("  - Latency: 80-300ms (depending on distance)");
    println!("  - Throughput: 3-12 writes/second");
    println!("\nKey Insight: Lease follows the workload!");
    println!("  - Morning: Asia holds leases");
    println!("  - Afternoon: Europe holds leases");
    println!("  - Evening: Americas hold leases");
    
    println!("\n=== The Lis Philosophy ===");
    println!("✓ With lease: \"You are king\" - microsecond writes");
    println!("✓ Without lease: Still writable via proxy - no readonly periods");
    println!("✓ Reads: Always fast from local cache");
    println!("✓ Automatic: Leases migrate to active regions");
}
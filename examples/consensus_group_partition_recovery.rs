use lis::rhc::hlc::{HLC, HLCTimestamp};
use lis::rhc::crdt::{LeaseStateCRDT, ActorId, CRDT};
use lis::rhc::leases::LeaseScope;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::path::PathBuf;

/// Simulated consensus group node with HLC and CRDT state
struct CGNode {
    id: ActorId,
    hlc: Arc<HLC>,
    lease_crdt: Arc<Mutex<LeaseStateCRDT>>,
    clock_skew_ms: i64,
}

impl CGNode {
    fn new(id: &str, clock_skew_ms: i64) -> Self {
        let actor_id = ActorId::new(id);
        
        // Create HLC with skewed clock
        let skew = clock_skew_ms;
        let clock_fn = move || {
            let real_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            
            if skew >= 0 {
                real_ms + skew as u64
            } else {
                real_ms - (-skew) as u64
            }
        };
        
        let hlc = Arc::new(HLC::new_with_clock(Box::new(clock_fn)));
        let lease_crdt = Arc::new(Mutex::new(LeaseStateCRDT::new(actor_id.clone())));
        
        Self {
            id: actor_id,
            hlc,
            lease_crdt,
            clock_skew_ms,
        }
    }
    
    fn request_lease(&self, scope: LeaseScope, duration_ms: u64) -> Result<(), String> {
        let now = self.hlc.now();
        let expires_at = HLCTimestamp::new(now.physical + duration_ms, 0);
        
        let mut crdt = self.lease_crdt.lock().unwrap();
        
        // Check if lease is available
        if let Some(info) = crdt.is_lease_valid(&scope, now) {
            return Err(format!("Lease held by {} until {}", info.holder.0, info.expires_at));
        }
        
        // Grant lease
        crdt.grant_lease(&scope, self.id.clone(), now, expires_at);
        println!("  {} granted lease at {} (expires {})", self.id.0, now, expires_at);
        Ok(())
    }
    
    fn sync_with(&self, other: &CGNode) {
        // Exchange HLC timestamps
        let my_ts = self.hlc.now();
        let other_ts = other.hlc.now();
        
        let _ = self.hlc.update(other_ts);
        let _ = other.hlc.update(my_ts);
        
        // Exchange CRDT states
        let my_crdt = self.lease_crdt.lock().unwrap().clone();
        let other_crdt = other.lease_crdt.lock().unwrap().clone();
        
        self.lease_crdt.lock().unwrap().merge(&other_crdt);
        other.lease_crdt.lock().unwrap().merge(&my_crdt);
        
        println!("  {} <-> {} synchronized (HLC: {} <-> {})", 
                 self.id.0, other.id.0, my_ts, other_ts);
    }
}

/// Simulated consensus group with multiple nodes
struct ConsensusGroup {
    name: String,
    nodes: Vec<Arc<CGNode>>,
}

impl ConsensusGroup {
    fn new(name: &str, node_configs: Vec<(&str, i64)>) -> Self {
        let nodes: Vec<_> = node_configs
            .into_iter()
            .map(|(id, skew)| Arc::new(CGNode::new(id, skew)))
            .collect();
        
        Self {
            name: name.to_string(),
            nodes,
        }
    }
    
    fn process_lease_request(&self, requester_id: &str, scope: LeaseScope, duration_ms: u64) -> Result<(), String> {
        println!("\n{} processing lease request from {}", self.name, requester_id);
        
        // Find majority of nodes
        let majority = (self.nodes.len() / 2) + 1;
        let mut successes = 0;
        let mut errors = Vec::new();
        
        // Each node processes the request
        for node in &self.nodes {
            match node.request_lease(scope.clone(), duration_ms) {
                Ok(_) => successes += 1,
                Err(e) => errors.push(format!("{}: {}", node.id.0, e)),
            }
        }
        
        if successes >= majority {
            println!("  Lease granted by majority ({}/{})", successes, self.nodes.len());
            
            // Sync all nodes to ensure consistency
            self.sync_all_nodes();
            Ok(())
        } else {
            Err(format!("Failed to get majority: {}", errors.join("; ")))
        }
    }
    
    fn sync_all_nodes(&self) {
        println!("  Synchronizing all nodes in {}...", self.name);
        
        // All-to-all sync (simplified - real system would use gossip)
        for i in 0..self.nodes.len() {
            for j in i+1..self.nodes.len() {
                self.nodes[i].sync_with(&self.nodes[j]);
            }
        }
    }
    
    fn show_state(&self) {
        println!("\n{} State:", self.name);
        for node in &self.nodes {
            let now = node.hlc.now();
            let crdt = node.lease_crdt.lock().unwrap();
            let active_leases = crdt.active_leases(now);
            
            println!("  {} (skew: {:+}ms, HLC: {})", 
                     node.id.0, node.clock_skew_ms, now);
            
            for (scope, info) in active_leases {
                println!("    Lease: {} held by {} until {}", 
                         scope, info.holder.0, info.expires_at);
            }
        }
    }
}

fn main() {
    println!("=== HLC + CG + CRDT Integration Demo ===");
    println!("Shows how consensus groups use HLC and CRDTs to maintain");
    println!("consistent lease state despite clock skew.\n");
    
    // Create a consensus group with nodes having different clock skews
    let cg = ConsensusGroup::new("DataCG", vec![
        ("cg1-node1", 0),       // Accurate clock
        ("cg1-node2", 30_000),  // 30 seconds fast
        ("cg1-node3", -20_000), // 20 seconds slow
    ]);
    
    println!("Initial consensus group state:");
    for node in &cg.nodes {
        println!("  {} clock skew: {:+}ms", node.id.0, node.clock_skew_ms);
    }
    
    // Test 1: Lease request with skewed clocks
    println!("\n--- Test 1: Lease Request with Clock Skew ---");
    let scope1 = LeaseScope::File(PathBuf::from("/data/file1.txt"));
    match cg.process_lease_request("client1", scope1.clone(), 30_000) {
        Ok(_) => println!("SUCCESS: Lease granted despite clock skew"),
        Err(e) => println!("ERROR: {}", e),
    }
    
    cg.show_state();
    
    // Wait a bit
    thread::sleep(Duration::from_millis(1000));
    
    // Test 2: Concurrent lease requests
    println!("\n--- Test 2: Concurrent Lease Requests ---");
    let scope2 = LeaseScope::File(PathBuf::from("/data/file2.txt"));
    
    // Simulate concurrent requests by having nodes process independently first
    println!("\nNodes process requests independently:");
    for node in &cg.nodes {
        let result = node.request_lease(scope2.clone(), 25_000);
        println!("  {} result: {:?}", node.id.0, result);
    }
    
    // Now sync to resolve conflicts
    println!("\nSyncing to resolve conflicts:");
    cg.sync_all_nodes();
    
    cg.show_state();
    
    // Test 3: Partition and merge
    println!("\n--- Test 3: Network Partition Simulation ---");
    
    // Create two partitions
    let partition1 = vec![cg.nodes[0].clone()];
    let partition2 = vec![cg.nodes[1].clone(), cg.nodes[2].clone()];
    
    println!("\nPartition 1: {}", partition1[0].id.0);
    println!("Partition 2: {}, {}", partition2[0].id.0, partition2[1].id.0);
    
    // Each partition grants different leases
    let scope3 = LeaseScope::File(PathBuf::from("/data/file3.txt"));
    let scope4 = LeaseScope::File(PathBuf::from("/data/file4.txt"));
    
    println!("\nPartition 1 grants lease for file3:");
    let _ = partition1[0].request_lease(scope3.clone(), 20_000);
    
    println!("\nPartition 2 grants lease for file4:");
    let _ = partition2[0].request_lease(scope4.clone(), 20_000);
    partition2[0].sync_with(&partition2[1]);
    
    // Show state before merge
    println!("\nState before partition merge:");
    for node in &cg.nodes {
        let crdt = node.lease_crdt.lock().unwrap();
        let now = node.hlc.now();
        let leases = crdt.active_leases(now);
        println!("  {} sees {} active leases", node.id.0, leases.len());
    }
    
    // Heal partition
    println!("\nHealing partition...");
    cg.sync_all_nodes();
    
    // Show final state
    cg.show_state();
    
    println!("\n=== Key Observations ===");
    println!("1. HLC ensures consistent timestamp ordering across nodes");
    println!("2. CRDTs allow lease state to be merged after partitions");
    println!("3. Clock skew doesn't break consensus - HLC handles it");
    println!("4. All nodes converge to the same state after synchronization");
}
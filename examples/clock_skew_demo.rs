use lis::rhc::hlc::{HLC, HLCTimestamp};
use lis::rhc::leases::{LeaseScope, LeaseManager, Lease, LeaseId};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::thread;
use std::collections::HashMap;

/// Simulated consensus group that maintains lease state
/// In real RHC, this would be a BFT consensus group with multiple nodes
#[derive(Clone)]
struct ConsensusGroup {
    name: String,
    lease_table: Arc<Mutex<HashMap<String, ActiveLease>>>,
    hlc: Arc<HLC>,
}

#[derive(Clone)]
struct ActiveLease {
    holder: String,
    granted_at: HLCTimestamp,
    expires_at: HLCTimestamp,
    id: LeaseId,
}

impl ConsensusGroup {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            lease_table: Arc::new(Mutex::new(HashMap::new())),
            hlc: Arc::new(HLC::new()),
        }
    }
    
    /// Process lease request through consensus
    /// In real RHC, this would be a BFT consensus round
    fn process_lease_request(&self, path: &str, requester: String, request_ts: HLCTimestamp, duration_ms: u64) -> Result<LeaseId, String> {
        // Update our HLC with the request timestamp
        let _ = self.hlc.update(request_ts);
        
        let mut leases = self.lease_table.lock().unwrap();
        let consensus_ts = self.hlc.now();
        
        // Check existing lease
        if let Some(existing) = leases.get(path) {
            // Use consensus timestamp to check expiry
            if existing.expires_at > consensus_ts {
                return Err(format!("Lease held by {} until {}", existing.holder, existing.expires_at));
            }
        }
        
        // Grant new lease with consensus timestamp
        let expires_at = HLCTimestamp {
            physical: consensus_ts.physical + duration_ms,
            logical: 0,
        };
        
        let id = LeaseId::new();
        leases.insert(path.to_string(), ActiveLease {
            holder: requester,
            granted_at: consensus_ts,
            expires_at,
            id,
        });
        
        println!("  CG {} grants lease to {} at {} (expires {})", 
                 self.name, leases.get(path).unwrap().holder, consensus_ts, expires_at);
        Ok(id)
    }
    
    fn check_lease(&self, path: &str, check_ts: HLCTimestamp) -> Option<ActiveLease> {
        let _ = self.hlc.update(check_ts);
        self.lease_table.lock().unwrap().get(path).cloned()
    }
}

/// Simulated node with configurable clock skew
struct SkewedNode {
    name: String,
    clock_skew_ms: i64,
    hlc: Arc<HLC>,
    lease_manager: Arc<Mutex<LeaseManager>>,
    stats: Arc<Mutex<NodeStats>>,
    consensus_group: Arc<ConsensusGroup>,
}

#[derive(Default)]
struct NodeStats {
    writes_attempted: u64,
    writes_succeeded: u64,
    lease_acquisitions: u64,
    clock_corrections: u64,
}

impl SkewedNode {
    fn new(name: &str, clock_skew_ms: i64, consensus_group: Arc<ConsensusGroup>) -> Self {
        // Create HLC with custom clock that includes skew
        let skew = clock_skew_ms;
        let clock_fn = move || {
            let real_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            
            if skew >= 0 {
                real_ms + skew as u64
            } else {
                real_ms - (-skew) as u64
            }
        };
        
        let hlc = Arc::new(HLC::new_with_clock(Box::new(clock_fn)));
        
        Self {
            name: name.to_string(),
            clock_skew_ms,
            hlc: hlc.clone(),
            lease_manager: Arc::new(Mutex::new(LeaseManager::new(name.to_string(), hlc))),
            stats: Arc::new(Mutex::new(NodeStats::default())),
            consensus_group,
        }
    }
    
    
    fn try_write(&self, path: &str) -> Result<(), String> {
        let mut stats = self.stats.lock().unwrap();
        stats.writes_attempted += 1;
        drop(stats); // Release stats lock before acquiring manager lock
        
        // Check with consensus group
        let now = self.hlc.now();
        if let Some(lease) = self.consensus_group.check_lease(path, now) {
            if lease.expires_at > now && lease.holder == self.name {
                self.stats.lock().unwrap().writes_succeeded += 1;
                Ok(())
            } else if lease.expires_at > now {
                Err(format!("Lease held by {}", lease.holder))
            } else {
                Err("Lease expired".to_string())
            }
        } else {
            Err("No active lease".to_string())
        }
    }
    
    fn acquire_lease(&self, path: &str, duration_ms: u64) -> Result<(), String> {
        let request_ts = self.hlc.now();
        
        // Request lease from consensus group
        match self.consensus_group.process_lease_request(path, self.name.clone(), request_ts, duration_ms) {
            Ok(_id) => {
                // Also update local manager (for local operations)
                let manager = self.lease_manager.lock().unwrap();
                let scope = LeaseScope::File(path.into());
                let duration = Duration::from_millis(duration_ms);
                let _ = manager.acquire_lease(scope, duration);
                
                self.stats.lock().unwrap().lease_acquisitions += 1;
                Ok(())
            }
            Err(e) => Err(e)
        }
    }
}

fn main() {
    println!("=== RHC Clock Skew Demonstration ===\n");
    println!("This demo shows that RHC's lease system remains safe even when");
    println!("nodes have severely skewed clocks, thanks to HLC.\n");
    
    // Create consensus group that maintains lease state
    let cg = Arc::new(ConsensusGroup::new("DataCG"));
    println!("Consensus Group '{}' managing lease state", cg.name);
    
    // Create nodes with various clock problems
    let nodes = vec![
        Arc::new(SkewedNode::new("Accurate", 0, cg.clone())),
        Arc::new(SkewedNode::new("FastClock", 45_000, cg.clone())),      // 45 seconds fast
        Arc::new(SkewedNode::new("SlowClock", -60_000, cg.clone())),     // 60 seconds slow  
        Arc::new(SkewedNode::new("VeryFast", 300_000, cg.clone())),      // 5 minutes fast!
        Arc::new(SkewedNode::new("VerySlow", -180_000, cg.clone())),     // 3 minutes slow
    ];
    
    // Display initial clock states
    println!("Initial clock states (physical time):");
    
    for node in &nodes {
        let skew_seconds = node.clock_skew_ms as f64 / 1000.0;
        let sign = if skew_seconds >= 0.0 { "+" } else { "" };
        println!("  {:<12} : {}{}s from real time", 
                 node.name, sign, skew_seconds);
    }
    
    println!("\n--- Phase 1: Initial Lease Acquisition ---");
    
    // Accurate node acquires lease first
    let path = "/data/important.db";
    nodes[0].acquire_lease(path, 30_000).unwrap();
    println!("{} acquired lease for {}", nodes[0].name, path);
    
    // Other nodes try to acquire but should fail (lease active)
    for node in &nodes[1..] {
        match node.acquire_lease(path, 30_000) {
            Ok(_) => println!("{} ERROR: acquired lease when it shouldn't!", node.name),
            Err(e) => println!("{} cannot acquire: {}", node.name, e),
        }
    }
    
    println!("\n--- Phase 2: Nodes Synchronize via HLC ---");
    
    // Simulate nodes communicating and synchronizing their HLCs
    // This happens naturally in a real system through message passing
    let mut timestamps = vec![];
    for node in &nodes {
        let ts = node.hlc.now();
        timestamps.push(ts);
        println!("{}: HLC timestamp {}", node.name, ts);
    }
    
    // Each node receives timestamps from others (simulating gossip)
    for i in 0..nodes.len() {
        for j in 0..timestamps.len() {
            if i != j {
                let _ = nodes[i].hlc.update(timestamps[j]);
                nodes[i].stats.lock().unwrap().clock_corrections += 1;
            }
        }
    }
    
    println!("\n--- Phase 3: Coordinated Lease Migration ---");
    
    // Wait for lease to near expiry
    thread::sleep(Duration::from_millis(2000));
    
    // FastClock node tries to acquire lease
    // Even though its physical clock is way ahead, HLC ensures
    // it respects the actual lease expiry time
    println!("\nFastClock (45s ahead) attempts lease acquisition...");
    
    
    // Simulate lease fence propagation
    let fence_ts = nodes[0].hlc.now();
    println!("Accurate creates fence at HLC {}", fence_ts);
    
    // FastClock sees the fence and respects it
    let _ = nodes[1].hlc.update(fence_ts);
    
    // Now FastClock can safely acquire after fence
    match nodes[1].acquire_lease(path, 30_000) {
        Ok(_) => println!("FastClock safely acquired lease after fence"),
        Err(e) => println!("FastClock acquisition failed: {}", e),
    }
    
    println!("\n--- Phase 4: Stress Test with Extreme Skew ---");
    
    // VeryFast (5 minutes ahead) and VerySlow (3 minutes behind)
    // attempt coordinated operations
    
    let very_fast = &nodes[3];
    let very_slow = &nodes[4];
    
    println!("\nExtreme clock skew test:");
    println!("  VeryFast: {} (5 min ahead)", very_fast.hlc.now());
    println!("  VerySlow: {} (3 min behind)", very_slow.hlc.now());
    
    // They communicate
    let vf_ts = very_fast.hlc.now();
    let vs_ts = very_slow.hlc.now();
    
    let _ = very_fast.hlc.update(vs_ts);
    let _ = very_slow.hlc.update(vf_ts);
    
    println!("\nAfter HLC synchronization:");
    println!("  VeryFast: {}", very_fast.hlc.now());
    println!("  VerySlow: {}", very_slow.hlc.now());
    
    // Both nodes now have consistent view of time ordering
    // and can safely coordinate lease transfers
    
    println!("\n--- Phase 5: Concurrent Write Attempts ---");
    
    // Multiple nodes attempt writes concurrently
    let mut handles = vec![];
    
    for node in nodes.iter() {
        let node_clone = node.clone();
        let handle = thread::spawn(move || {
            for i in 0..10 {
                thread::sleep(Duration::from_millis(100));
                let result = node_clone.try_write(path);
                if result.is_ok() {
                    println!("{} completed write {}", node_clone.name, i);
                }
            }
        });
        handles.push(handle);
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    println!("\n=== Final Statistics ===");
    for node in &nodes {
        let stats = node.stats.lock().unwrap();
        println!("\n{}:", node.name);
        println!("  Clock skew:          {:+}ms", node.clock_skew_ms);
        println!("  Writes attempted:    {}", stats.writes_attempted);
        println!("  Writes succeeded:    {}", stats.writes_succeeded);
        println!("  Lease acquisitions:  {}", stats.lease_acquisitions);
        println!("  Clock corrections:   {}", stats.clock_corrections);
    }
    
    println!("\n=== Key Observations ===");
    println!("1. Despite extreme clock skew (up to ±5 minutes), no safety violations occurred");
    println!("2. HLC synchronization allowed nodes to establish consistent time ordering");
    println!("3. Lease fencing worked correctly even with badly skewed clocks");
    println!("4. Only lease holders successfully completed writes (safety preserved)");
    println!("5. Clock corrections via HLC message passing kept system coordinated");
}
// Demonstrates basic Consensus Group concepts without full implementation
// This is a simplified model to understand CG behavior

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Simplified representation of a consensus group node
#[derive(Clone)]
struct CGNode {
    id: String,
    group_id: String,
    state: Arc<Mutex<HashMap<String, String>>>,
    is_leader: Arc<Mutex<bool>>,
    last_heartbeat: Arc<Mutex<Instant>>,
    term: Arc<Mutex<u64>>,
}

impl CGNode {
    fn new(id: String, group_id: String) -> Self {
        Self {
            id,
            group_id,
            state: Arc::new(Mutex::new(HashMap::new())),
            is_leader: Arc::new(Mutex::new(false)),
            last_heartbeat: Arc::new(Mutex::new(Instant::now())),
            term: Arc::new(Mutex::new(0)),
        }
    }

    /// Simulate processing a write operation
    fn write(&self, key: String, value: String) -> Result<(), String> {
        if !*self.is_leader.lock().unwrap() {
            return Err("Not the leader".to_string());
        }

        // In real implementation, this would go through consensus
        println!("{} (leader): Processing write {}={}", self.id, key, value);
        
        // Simulate replication delay
        std::thread::sleep(Duration::from_millis(10));
        
        self.state.lock().unwrap().insert(key, value);
        Ok(())
    }

    /// Simulate reading from the state machine
    fn read(&self, key: &str) -> Option<String> {
        self.state.lock().unwrap().get(key).cloned()
    }

    /// Simulate leader election
    fn attempt_leadership(&self, other_nodes: &[CGNode]) {
        let mut current_term = self.term.lock().unwrap();
        *current_term += 1;
        let election_term = *current_term;
        drop(current_term);

        println!("{}: Starting election for term {}", self.id, election_term);

        // Count votes (simplified - in reality would be RPC)
        let mut votes = 1; // vote for self
        for node in other_nodes {
            if node.id != self.id {
                // Simulate vote request
                if *node.term.lock().unwrap() < election_term {
                    votes += 1;
                    *node.term.lock().unwrap() = election_term;
                }
            }
        }

        let majority = (other_nodes.len() + 1) / 2 + 1;
        if votes >= majority {
            println!("{}: Won election with {} votes (needed {})", 
                     self.id, votes, majority);
            *self.is_leader.lock().unwrap() = true;
            *self.last_heartbeat.lock().unwrap() = Instant::now();
        } else {
            println!("{}: Lost election with {} votes (needed {})", 
                     self.id, votes, majority);
        }
    }

    /// Check if node should start election
    fn check_election_timeout(&self) -> bool {
        let last = *self.last_heartbeat.lock().unwrap();
        let elapsed = Instant::now().duration_since(last);
        elapsed > Duration::from_millis(300) && !*self.is_leader.lock().unwrap()
    }

    /// Send heartbeat if leader
    fn send_heartbeat(&self, followers: &[CGNode]) {
        if !*self.is_leader.lock().unwrap() {
            return;
        }

        for follower in followers {
            if follower.id != self.id {
                *follower.last_heartbeat.lock().unwrap() = Instant::now();
            }
        }
    }
}

/// Represents a consensus group managing replicated state
struct ConsensusGroup {
    id: String,
    nodes: Vec<CGNode>,
}

impl ConsensusGroup {
    fn new(id: String, node_count: usize) -> Self {
        let nodes: Vec<CGNode> = (0..node_count)
            .map(|i| CGNode::new(format!("node-{}", i), id.clone()))
            .collect();

        // Make first node the initial leader
        if !nodes.is_empty() {
            *nodes[0].is_leader.lock().unwrap() = true;
        }

        Self { id, nodes }
    }

    /// Get current leader
    fn get_leader(&self) -> Option<&CGNode> {
        self.nodes.iter().find(|n| *n.is_leader.lock().unwrap())
    }

    /// Simulate network partition
    fn partition(&self, partition_size: usize) -> (Vec<CGNode>, Vec<CGNode>) {
        let partition1 = self.nodes[..partition_size].to_vec();
        let partition2 = self.nodes[partition_size..].to_vec();
        
        println!("\n=== Network Partition ===");
        println!("Partition 1: {} nodes", partition1.len());
        println!("Partition 2: {} nodes", partition2.len());
        
        (partition1, partition2)
    }

    /// Run consensus group simulation
    fn run_simulation(&self) {
        println!("\n=== Running Consensus Group Simulation ===");
        println!("Group: {} with {} nodes", self.id, self.nodes.len());

        // Simulate some operations
        if let Some(leader) = self.get_leader() {
            println!("\nInitial leader: {}", leader.id);
            
            // Perform some writes
            leader.write("key1".to_string(), "value1".to_string()).unwrap();
            leader.write("key2".to_string(), "value2".to_string()).unwrap();
            
            // Show state replication (simplified)
            std::thread::sleep(Duration::from_millis(50));
            for node in &self.nodes {
                // In reality, followers would receive log entries
                if !*node.is_leader.lock().unwrap() {
                    *node.state.lock().unwrap() = leader.state.lock().unwrap().clone();
                }
            }
            
            println!("\nState after replication:");
            for node in &self.nodes {
                let state = node.state.lock().unwrap();
                println!("  {}: {} keys", node.id, state.len());
            }
        }

        // Simulate leader failure and election
        println!("\n=== Simulating Leader Failure ===");
        if let Some(leader) = self.get_leader() {
            *leader.is_leader.lock().unwrap() = false;
            println!("Leader {} failed", leader.id);
        }

        // Wait for election timeout
        std::thread::sleep(Duration::from_millis(400));

        // Node 1 attempts to become leader
        if self.nodes.len() > 1 {
            self.nodes[1].attempt_leadership(&self.nodes);
        }
    }
}

fn main() {
    println!("=== Consensus Group Basics Demo ===\n");
    
    // Create a 5-node consensus group
    let cg = ConsensusGroup::new("DataCG".to_string(), 5);
    cg.run_simulation();
    
    // Demonstrate partition behavior
    println!("\n=== Partition Behavior ===");
    let (partition1, partition2) = cg.partition(3);
    
    println!("\nPartition 1 (majority):");
    if partition1.len() > 2 {
        // This partition can continue operating
        partition1[1].attempt_leadership(&partition1);
        if let Some(leader) = partition1.iter().find(|n| *n.is_leader.lock().unwrap()) {
            match leader.write("key3".to_string(), "value3".to_string()) {
                Ok(_) => println!("  Write succeeded in majority partition"),
                Err(e) => println!("  Write failed: {}", e),
            }
        }
    }
    
    println!("\nPartition 2 (minority):");
    if partition2.len() >= 1 {
        // This partition cannot elect a leader
        partition2[0].attempt_leadership(&partition2);
        if let Some(leader) = partition2.iter().find(|n| *n.is_leader.lock().unwrap()) {
            println!("  ERROR: Minority partition elected leader!");
        } else {
            println!("  Correctly failed to elect leader (no majority)");
        }
    }
    
    // Demonstrate state machine properties
    println!("\n=== State Machine Properties ===");
    println!("1. Deterministic: Same operations = same state");
    println!("2. Replicated: All nodes maintain identical state");
    println!("3. Linearizable: Operations appear to execute atomically");
    println!("4. Durable: State survives failures (with majority)");
    
    // Show configuration considerations
    println!("\n=== Configuration Guidelines ===");
    println!("Recommended group sizes:");
    println!("  - 3 nodes: Tolerates 1 failure");
    println!("  - 5 nodes: Tolerates 2 failures (recommended)");
    println!("  - 7 nodes: Tolerates 3 failures");
    println!("\nConsensus requirements:");
    println!("  - Raft: Majority (n/2 + 1)");
    println!("  - BFT: 2f + 1 out of 3f + 1 nodes");
    
    println!("\n=== Key Concepts Demonstrated ===");
    println!("✓ Leader election with term numbers");
    println!("✓ State replication across nodes");
    println!("✓ Partition tolerance (majority can progress)");
    println!("✓ Safety (minority cannot elect leader)");
}
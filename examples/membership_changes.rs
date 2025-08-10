// Demonstrates consensus group membership changes
// Shows join, leave, and failure handling protocols

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

#[derive(Debug, Clone, PartialEq)]
enum NodeState {
    Joining,
    Active, 
    Leaving,
    Failed,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    id: String,
    state: NodeState,
    last_heartbeat: Instant,
    joined_at: Instant,
}

#[derive(Debug, Clone)]
struct MembershipConfig {
    epoch: u64,
    members: HashMap<String, NodeInfo>,
    leader: Option<String>,
}

/// Simulates a consensus group with membership management
struct ConsensusGroup {
    id: String,
    config: Arc<Mutex<MembershipConfig>>,
    heartbeat_timeout: Duration,
}

impl ConsensusGroup {
    fn new(id: String, initial_nodes: Vec<String>) -> Self {
        let mut members = HashMap::new();
        let now = Instant::now();
        
        for node_id in initial_nodes.iter() {
            members.insert(node_id.clone(), NodeInfo {
                id: node_id.clone(),
                state: NodeState::Active,
                last_heartbeat: now,
                joined_at: now,
            });
        }

        let config = MembershipConfig {
            epoch: 1,
            members,
            leader: initial_nodes.first().cloned(),
        };

        Self {
            id,
            config: Arc::new(Mutex::new(config)),
            heartbeat_timeout: Duration::from_millis(500),
        }
    }

    /// Process a node join request
    fn handle_join(&self, new_node_id: String) -> Result<(), String> {
        println!("\n=== Node Join Protocol ===");
        println!("Node {} requesting to join group {}", new_node_id, self.id);

        let mut config = self.config.lock().unwrap();
        
        // Check if node already exists
        if config.members.contains_key(&new_node_id) {
            return Err("Node already in group".to_string());
        }

        println!("Step 1: Validation");
        // Simulate validation checks
        thread::sleep(Duration::from_millis(50));
        println!("  ✓ Identity verified");
        println!("  ✓ Resources adequate");
        println!("  ✓ No conflicts detected");

        // Add node in Joining state
        config.members.insert(new_node_id.clone(), NodeInfo {
            id: new_node_id.clone(),
            state: NodeState::Joining,
            last_heartbeat: Instant::now(),
            joined_at: Instant::now(),
        });

        println!("\nStep 2: State Transfer");
        println!("  → Sending snapshot to {}", new_node_id);
        thread::sleep(Duration::from_millis(100));
        println!("  ✓ Snapshot acknowledged");

        println!("\nStep 3: Joint Consensus");
        println!("  Old config: {} members", config.members.len() - 1);
        println!("  New config: {} members", config.members.len());
        
        // Simulate joint consensus
        config.epoch += 1;
        println!("  ✓ Configuration {} committed", config.epoch);

        // Activate the node
        if let Some(node) = config.members.get_mut(&new_node_id) {
            node.state = NodeState::Active;
        }

        println!("\nStep 4: Activation");
        println!("  ✓ Node {} is now active", new_node_id);
        
        self.print_membership(&config);
        Ok(())
    }

    /// Process a planned node departure
    fn handle_leave(&self, node_id: String) -> Result<(), String> {
        println!("\n=== Node Leave Protocol ===");
        println!("Node {} requesting to leave group {}", node_id, self.id);

        let mut config = self.config.lock().unwrap();
        
        // Check if node exists
        let node = config.members.get_mut(&node_id)
            .ok_or("Node not in group")?;
        
        if node.state != NodeState::Active {
            return Err("Node not active".to_string());
        }

        println!("Step 1: Announcement");
        node.state = NodeState::Leaving;
        println!("  ✓ Node {} marked as leaving", node_id);

        println!("\nStep 2: Load Migration");
        println!("  → Redistributing data from {}", node_id);
        thread::sleep(Duration::from_millis(150));
        println!("  ✓ Data migration complete");

        println!("\nStep 3: Configuration Change");
        config.epoch += 1;
        config.members.remove(&node_id);
        
        // Select new leader if needed
        if config.leader.as_ref() == Some(&node_id) {
            config.leader = config.members.keys().next().cloned();
            println!("  → New leader selected: {:?}", config.leader);
        }
        
        println!("  ✓ Configuration {} committed", config.epoch);
        
        self.print_membership(&config);
        Ok(())
    }

    /// Detect and handle node failures
    fn detect_failures(&self) {
        println!("\n=== Failure Detection ===");
        
        let mut config = self.config.lock().unwrap();
        let now = Instant::now();
        let mut failed_nodes = Vec::new();

        for (node_id, node_info) in config.members.iter() {
            if node_info.state == NodeState::Active {
                let elapsed = now.duration_since(node_info.last_heartbeat);
                if elapsed > self.heartbeat_timeout {
                    println!("Node {} missed heartbeats ({:?} since last)",
                            node_id, elapsed);
                    failed_nodes.push(node_id.clone());
                }
            }
        }

        for node_id in &failed_nodes {
            println!("\nHandling failure of node {}", node_id);
            
            // Mark as failed
            if let Some(node) = config.members.get_mut(node_id) {
                node.state = NodeState::Failed;
            }

            // Simulate consensus on failure
            thread::sleep(Duration::from_millis(100));
            println!("  ✓ Majority agrees {} has failed", node_id);
            
            // Remove from configuration
            config.epoch += 1;
            config.members.remove(node_id);
            
            // Handle leader failure
            if config.leader.as_ref() == Some(node_id) {
                // Simple leader selection - first active node
                config.leader = config.members.iter()
                    .find(|(_, info)| info.state == NodeState::Active)
                    .map(|(id, _)| id.clone());
                println!("  → New leader elected: {:?}", config.leader);
            }
            
            println!("  ✓ Configuration {} committed without {}", 
                    config.epoch, node_id);
        }

        if !failed_nodes.is_empty() {
            self.print_membership(&config);
        }
    }

    /// Simulate split-brain scenario
    fn simulate_partition(&self) {
        println!("\n=== Network Partition Scenario ===");
        
        let config = self.config.lock().unwrap();
        let active_nodes: Vec<_> = config.members.iter()
            .filter(|(_, info)| info.state == NodeState::Active)
            .map(|(id, _)| id.clone())
            .collect();
        
        let total = active_nodes.len();
        let partition_size = total / 2;
        
        println!("Total nodes: {}", total);
        println!("Partition A: {} nodes", partition_size);
        println!("Partition B: {} nodes", total - partition_size);
        
        // Check if each partition can make progress
        let partition_a_can_progress = partition_size > total / 2;
        let partition_b_can_progress = (total - partition_size) > total / 2;
        
        println!("\nPartition A (size {}): {}", 
                partition_size,
                if partition_a_can_progress { "CAN make progress ✓" } 
                else { "CANNOT make progress ✗" });
        
        println!("Partition B (size {}): {}", 
                total - partition_size,
                if partition_b_can_progress { "CAN make progress ✓" } 
                else { "CANNOT make progress ✗" });
        
        if !partition_a_can_progress && !partition_b_can_progress {
            println!("\n⚠️  SPLIT BRAIN: No partition has majority!");
            println!("   Group is unavailable until partition heals");
        }
    }

    /// Update heartbeat for a node
    fn update_heartbeat(&self, node_id: &str) {
        let mut config = self.config.lock().unwrap();
        if let Some(node) = config.members.get_mut(node_id) {
            node.last_heartbeat = Instant::now();
        }
    }

    /// Print current membership
    fn print_membership(&self, config: &MembershipConfig) {
        println!("\nCurrent Membership (Epoch {}):", config.epoch);
        println!("  Leader: {:?}", config.leader);
        println!("  Members:");
        for (id, info) in &config.members {
            println!("    - {}: {:?}", id, info.state);
        }
        println!("  Total: {} nodes", config.members.len());
    }
}

fn main() {
    println!("=== Consensus Group Membership Demo ===");

    // Create a 3-node group
    let initial_nodes = vec![
        "node-1".to_string(),
        "node-2".to_string(),
        "node-3".to_string(),
    ];
    
    let group = ConsensusGroup::new("DataCG".to_string(), initial_nodes);
    
    // Show initial state
    {
        let config = group.config.lock().unwrap();
        group.print_membership(&config);
    }

    // Demonstrate node join
    thread::sleep(Duration::from_millis(200));
    match group.handle_join("node-4".to_string()) {
        Ok(_) => println!("Join successful"),
        Err(e) => println!("Join failed: {}", e),
    }

    // Demonstrate node leave
    thread::sleep(Duration::from_millis(200));
    match group.handle_leave("node-2".to_string()) {
        Ok(_) => println!("Leave successful"),
        Err(e) => println!("Leave failed: {}", e),
    }

    // Update some heartbeats
    group.update_heartbeat("node-1");
    group.update_heartbeat("node-4");
    // Don't update node-3 to simulate failure

    // Wait for heartbeat timeout
    thread::sleep(Duration::from_millis(600));
    
    // Detect failures
    group.detect_failures();

    // Demonstrate partition scenarios
    thread::sleep(Duration::from_millis(200));
    group.simulate_partition();

    // Show configuration evolution
    println!("\n=== Configuration Evolution ===");
    println!("The group evolved through {} epochs", 
            group.config.lock().unwrap().epoch);
    
    // Best practices
    println!("\n=== Best Practices ===");
    println!("1. Odd number of nodes (3, 5, 7) for clear majorities");
    println!("2. Gradual membership changes (one at a time)");
    println!("3. Health checks before adding nodes");
    println!("4. Graceful shutdown when possible");
    println!("5. Monitor membership change frequency");
}
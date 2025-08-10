// Demonstrates State Machine Replication (SMR) concepts
// Shows how consensus groups maintain consistent state across replicas

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Commands that can be applied to the state machine
#[derive(Debug, Clone)]
enum Command {
    Set { key: String, value: String },
    Delete { key: String },
    Increment { key: String, amount: i64 },
}

/// Responses from state machine operations
#[derive(Debug, Clone)]
enum Response {
    Ok,
    Value(String),
    Error(String),
}

/// Log entry in the replicated log
#[derive(Debug, Clone)]
struct LogEntry {
    index: u64,
    term: u64,
    command: Command,
}

/// Abstract state machine interface
trait StateMachine: Send + Sync {
    fn apply(&mut self, command: Command) -> Response;
    fn snapshot(&self) -> Vec<u8>;
    fn restore(&mut self, snapshot: &[u8]);
}

/// Key-value store state machine implementation
struct KVStateMachine {
    data: HashMap<String, String>,
    counters: HashMap<String, i64>,
}

impl KVStateMachine {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
            counters: HashMap::new(),
        }
    }
}

impl StateMachine for KVStateMachine {
    fn apply(&mut self, command: Command) -> Response {
        match command {
            Command::Set { key, value } => {
                self.data.insert(key, value);
                Response::Ok
            }
            Command::Delete { key } => {
                if self.data.remove(&key).is_some() {
                    Response::Ok
                } else {
                    Response::Error("Key not found".to_string())
                }
            }
            Command::Increment { key, amount } => {
                let counter = self.counters.entry(key).or_insert(0);
                *counter += amount;
                Response::Value(counter.to_string())
            }
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        // Simplified - in reality would use proper serialization
        format!("{:?}|{:?}", self.data, self.counters).into_bytes()
    }

    fn restore(&mut self, _snapshot: &[u8]) {
        // Simplified - would deserialize in reality
        println!("Restoring from snapshot");
    }
}

/// Replica in the consensus group
struct Replica {
    id: String,
    state_machine: Box<dyn StateMachine>,
    log: Vec<LogEntry>,
    commit_index: u64,
    last_applied: u64,
}

impl Replica {
    fn new(id: String) -> Self {
        Self {
            id,
            state_machine: Box::new(KVStateMachine::new()),
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
        }
    }

    /// Append entry to log (would be replicated in real implementation)
    fn append_log_entry(&mut self, entry: LogEntry) {
        println!("{}: Appending log entry {:?}", self.id, entry);
        self.log.push(entry);
    }

    /// Apply committed entries to state machine
    fn apply_committed_entries(&mut self) {
        while self.last_applied < self.commit_index {
            self.last_applied += 1;
            if let Some(entry) = self.log.get(self.last_applied as usize - 1) {
                let response = self.state_machine.apply(entry.command.clone());
                println!("{}: Applied entry {} -> {:?}", 
                         self.id, self.last_applied, response);
            }
        }
    }

    /// Update commit index (after replication)
    fn update_commit_index(&mut self, new_commit_index: u64) {
        if new_commit_index > self.commit_index {
            self.commit_index = new_commit_index;
            self.apply_committed_entries();
        }
    }
}

/// Demonstrates various state machine replication scenarios
fn demonstrate_smr() {
    println!("=== State Machine Replication Demo ===\n");

    // Create replicas
    let mut replicas = vec![
        Replica::new("replica-1".to_string()),
        Replica::new("replica-2".to_string()),
        Replica::new("replica-3".to_string()),
    ];

    // Scenario 1: Normal replication
    println!("--- Scenario 1: Normal Replication ---");
    let commands = vec![
        Command::Set { 
            key: "user:123".to_string(), 
            value: "Alice".to_string() 
        },
        Command::Set { 
            key: "user:456".to_string(), 
            value: "Bob".to_string() 
        },
        Command::Increment { 
            key: "counter:visits".to_string(), 
            amount: 1 
        },
    ];

    // Leader receives commands and replicates
    for (index, cmd) in commands.iter().enumerate() {
        let entry = LogEntry {
            index: (index + 1) as u64,
            term: 1,
            command: cmd.clone(),
        };

        // All replicas append to log
        for replica in &mut replicas {
            replica.append_log_entry(entry.clone());
        }

        // Simulate majority acknowledgment and commit
        for replica in &mut replicas {
            replica.update_commit_index((index + 1) as u64);
        }
    }

    // Scenario 2: Follower catch-up
    println!("\n--- Scenario 2: Follower Catch-up ---");
    let mut new_replica = Replica::new("replica-4".to_string());
    println!("New replica joining with empty log");

    // Send snapshot to catch up (simplified)
    let snapshot = replicas[0].state_machine.snapshot();
    new_replica.state_machine.restore(&snapshot);
    new_replica.last_applied = 3;
    new_replica.commit_index = 3;
    println!("Replica-4 restored from snapshot at index 3");

    // Scenario 3: Divergent logs (after leader change)
    println!("\n--- Scenario 3: Log Divergence and Reconciliation ---");
    println!("Replica-1 was isolated and has uncommitted entries:");
    
    let divergent_entry = LogEntry {
        index: 4,
        term: 1,  // Old term
        command: Command::Set {
            key: "conflict".to_string(),
            value: "old-value".to_string(),
        },
    };
    replicas[0].append_log_entry(divergent_entry);

    println!("\nNew leader (replica-2) has different entry:");
    let correct_entry = LogEntry {
        index: 4,
        term: 2,  // New term
        command: Command::Set {
            key: "conflict".to_string(),
            value: "new-value".to_string(),
        },
    };

    // Replica-1 must truncate and follow new leader
    println!("Replica-1 truncates log and accepts new leader's entry");
    replicas[0].log.pop();  // Remove divergent entry
    replicas[0].append_log_entry(correct_entry.clone());

    // All replicas now have consistent logs
    for replica in &mut replicas[1..3] {
        replica.append_log_entry(correct_entry.clone());
    }

    // Commit the reconciled entry
    for replica in &mut replicas[0..3] {
        replica.update_commit_index(4);
    }

    // Scenario 4: Deterministic execution
    println!("\n--- Scenario 4: Deterministic Execution ---");
    println!("All replicas execute same commands in same order:");
    
    let test_commands = vec![
        Command::Set { 
            key: "x".to_string(), 
            value: "10".to_string() 
        },
        Command::Increment { 
            key: "x".to_string(), 
            amount: 5 
        },
        Command::Increment { 
            key: "x".to_string(), 
            amount: -3 
        },
    ];

    // Create fresh replicas for this test
    let mut test_replicas = vec![
        Replica::new("test-1".to_string()),
        Replica::new("test-2".to_string()),
        Replica::new("test-3".to_string()),
    ];

    // Apply commands to all replicas
    for (index, cmd) in test_commands.iter().enumerate() {
        let entry = LogEntry {
            index: (index + 1) as u64,
            term: 1,
            command: cmd.clone(),
        };

        for replica in &mut test_replicas {
            replica.append_log_entry(entry.clone());
            replica.update_commit_index((index + 1) as u64);
        }
    }

    println!("\nAll replicas have identical state after applying same log");

    // Key properties demonstrated
    println!("\n=== Key SMR Properties Demonstrated ===");
    println!("1. **Agreement**: All replicas apply same commands in same order");
    println!("2. **Integrity**: Commands are applied exactly once");
    println!("3. **Total Order**: All replicas see same sequence");
    println!("4. **Durability**: Committed entries survive failures");
    println!("5. **Determinism**: Same log produces same state");

    // Performance considerations
    println!("\n=== Performance Considerations ===");
    println!("- Batching: Group multiple commands per round-trip");
    println!("- Pipelining: Send next batch before previous commits");
    println!("- Snapshots: Compress log periodically");
    println!("- Read leases: Serve reads without consensus");
    
    // Safety considerations
    println!("\n=== Safety Considerations ===");
    println!("- Never commit entries from previous terms");
    println!("- Ensure log matching before replication");
    println!("- Check term number on every RPC");
    println!("- Persist state before responding to RPCs");
}

fn main() {
    demonstrate_smr();
}
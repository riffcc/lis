// Real demo showing write latency at different consistency levels

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::time::{Duration, Instant};
use std::thread;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
struct StorageNode {
    id: String,
    location: String,
    storage_type: StorageType,
    latency_us: u64,  // microseconds to this node
}

#[derive(Debug, Clone, Copy)]
enum StorageType {
    NVMe,
    SSD, 
    HDD,
}

#[derive(Debug)]
struct WriteRequest {
    id: u64,
    size_bytes: usize,
    timestamp: Instant,
}

#[derive(Debug)]
struct WriteAck {
    request_id: u64,
    node_id: String,
    ack_time: Instant,
}

#[derive(Debug, Clone, Copy)]
enum ConsistencyMode {
    FastAck,      // ACK after 1 local NVMe
    Balanced,     // ACK after 3 nodes in same rack
    Safe,         // ACK after geographic distribution
}

struct WriteCoordinator {
    nodes: Vec<StorageNode>,
    mode: ConsistencyMode,
    pending_writes: Arc<Mutex<VecDeque<WriteRequest>>>,
    ack_receiver: Receiver<WriteAck>,
}

impl WriteCoordinator {
    fn new(mode: ConsistencyMode, ack_receiver: Receiver<WriteAck>) -> Self {
        // Set up realistic nodes
        let nodes = vec![
            StorageNode {
                id: "london-nvme-1".to_string(),
                location: "london-mini".to_string(),
                storage_type: StorageType::NVMe,
                latency_us: 500,  // 0.5ms
            },
            StorageNode {
                id: "london-ssd-1".to_string(),
                location: "london-main".to_string(),
                storage_type: StorageType::SSD,
                latency_us: 2000,  // 2ms (same rack)
            },
            StorageNode {
                id: "london-ssd-2".to_string(),
                location: "london-main".to_string(),
                storage_type: StorageType::SSD,
                latency_us: 2000,  // 2ms (same rack)
            },
            StorageNode {
                id: "london-hdd-1".to_string(),
                location: "london-secondary".to_string(),
                storage_type: StorageType::HDD,
                latency_us: 5000,  // 5ms (cross-london)
            },
            StorageNode {
                id: "perth-hdd-1".to_string(),
                location: "perth".to_string(),
                storage_type: StorageType::HDD,
                latency_us: 250000,  // 250ms
            },
        ];

        Self {
            nodes,
            mode,
            pending_writes: Arc::new(Mutex::new(VecDeque::new())),
            ack_receiver,
        }
    }

    fn submit_write(&self, size_bytes: usize) -> (u64, Instant) {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let now = Instant::now();
        
        let request = WriteRequest {
            id,
            size_bytes,
            timestamp: now,
        };

        self.pending_writes.lock().unwrap().push_back(request);
        (id, now)
    }

    fn wait_for_acks(&self, request_id: u64, start_time: Instant) -> Duration {
        let mut acks = Vec::new();
        
        // Collect ACKs based on consistency mode requirements
        let required_acks = match self.mode {
            ConsistencyMode::FastAck => 1,
            ConsistencyMode::Balanced => 3,
            ConsistencyMode::Safe => 2,  // At least one from each site
        };

        while acks.len() < required_acks {
            if let Ok(ack) = self.ack_receiver.recv() {
                if ack.request_id == request_id {
                    acks.push(ack);
                    
                    // Check if we've met our consistency requirements
                    match self.mode {
                        ConsistencyMode::FastAck => {
                            // Any ACK is sufficient
                            if acks.len() >= 1 {
                                return acks[0].ack_time.duration_since(start_time);
                            }
                        }
                        ConsistencyMode::Balanced => {
                            // Need 3 ACKs from same location
                            let london_acks = acks.iter()
                                .filter(|a| a.node_id.starts_with("london"))
                                .count();
                            if london_acks >= 3 {
                                return acks.last().unwrap().ack_time.duration_since(start_time);
                            }
                        }
                        ConsistencyMode::Safe => {
                            // Need at least one ACK from different sites
                            let has_london = acks.iter().any(|a| a.node_id.starts_with("london"));
                            let has_perth = acks.iter().any(|a| a.node_id.starts_with("perth"));
                            if has_london && has_perth {
                                return acks.last().unwrap().ack_time.duration_since(start_time);
                            }
                        }
                    }
                }
            }
        }
        
        Instant::now().duration_since(start_time)
    }
}

fn simulate_storage_node(
    node: StorageNode,
    pending_writes: Arc<Mutex<VecDeque<WriteRequest>>>,
    ack_sender: Sender<WriteAck>,
) {
    thread::spawn(move || {
        loop {
            // Check for pending writes
            let write = {
                let mut queue = pending_writes.lock().unwrap();
                queue.front().cloned()
            };

            if let Some(write) = write {
                // Simulate write latency
                thread::sleep(Duration::from_micros(node.latency_us));
                
                // Send ACK
                let ack = WriteAck {
                    request_id: write.id,
                    node_id: node.id.clone(),
                    ack_time: Instant::now(),
                };
                
                let _ = ack_sender.send(ack);
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
    });
}

fn main() {
    println!("=== Real Write Latency Demo ===\n");
    println!("Configuration: Write 1MB to /data/file.dat");
    println!("Nodes:");
    println!("  - london-nvme-1   (0.5ms latency)");
    println!("  - london-ssd-1/2  (2ms latency, same rack)");
    println!("  - london-hdd-1    (5ms latency, different site)");
    println!("  - perth-hdd-1     (250ms latency)\n");

    // Test each consistency mode
    for mode in [ConsistencyMode::FastAck, ConsistencyMode::Balanced, ConsistencyMode::Safe] {
        let (ack_sender, ack_receiver) = channel();
        let coordinator = WriteCoordinator::new(mode, ack_receiver);
        
        // Start storage node simulators
        for node in coordinator.nodes.clone() {
            simulate_storage_node(
                node,
                coordinator.pending_writes.clone(),
                ack_sender.clone(),
            );
        }

        // Perform write
        let mode_name = match mode {
            ConsistencyMode::FastAck => "Fast ACK Mode (1 local NVMe)",
            ConsistencyMode::Balanced => "Balanced Mode (3 nodes same rack)",
            ConsistencyMode::Safe => "Safe Mode (geographic distribution)",
        };
        
        println!("Testing {}", mode_name);
        let (request_id, start_time) = coordinator.submit_write(1024 * 1024);
        let latency = coordinator.wait_for_acks(request_id, start_time);
        
        println!("  Write acknowledged in: {:?}", latency);
        
        match mode {
            ConsistencyMode::FastAck => {
                println!("  Risk: Node failure before replication = data loss");
            }
            ConsistencyMode::Balanced => {
                println!("  Protection: Survives any single node failure");
            }
            ConsistencyMode::Safe => {
                println!("  Protection: Survives entire site failure");
            }
        }
        println!();
        
        // Brief pause between tests
        thread::sleep(Duration::from_millis(100));
    }

    println!("Summary:");
    println!("- Fast ACK provides lowest latency but risks data loss");
    println!("- Balanced mode protects against node failures with minimal latency");
    println!("- Safe mode ensures geographic distribution at cost of latency");
}
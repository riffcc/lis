use crate::{
    node::{NodeRole, RhcNode},
    storage::InMemoryStorage,
    NodeId,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct LatencySimulator {
    pub local_latency_us: u64,
    pub regional_latency_ms: u64,
    pub global_latency_ms: u64,
}

impl Default for LatencySimulator {
    fn default() -> Self {
        Self {
            local_latency_us: 100,      // 100 microseconds for local
            regional_latency_ms: 5,      // 5ms for regional
            global_latency_ms: 100,      // 100ms for global
        }
    }
}

impl LatencySimulator {
    pub async fn simulate_local(&self) {
        sleep(Duration::from_micros(self.local_latency_us)).await;
    }
    
    pub async fn simulate_regional(&self) {
        sleep(Duration::from_millis(self.regional_latency_ms)).await;
    }
    
    pub async fn simulate_global(&self) {
        sleep(Duration::from_millis(self.global_latency_ms)).await;
    }
    
    pub async fn simulate_for_level(&self, level: u8) {
        match level {
            0 => self.simulate_local().await,
            1 | 2 => self.simulate_regional().await,
            3 => self.simulate_global().await,
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkSimulator {
    nodes: Vec<Arc<RhcNode>>,
    channels: dashmap::DashMap<(NodeId, NodeId), mpsc::UnboundedSender<crate::message::Message>>,
    latency: LatencySimulator,
    partition_map: dashmap::DashMap<(NodeId, NodeId), bool>,
}

impl NetworkSimulator {
    pub fn new(latency: LatencySimulator) -> Self {
        Self {
            nodes: Vec::new(),
            channels: dashmap::DashMap::new(),
            latency,
            partition_map: dashmap::DashMap::new(),
        }
    }
    
    pub fn add_node(&mut self, node: Arc<RhcNode>) {
        self.nodes.push(node);
    }
    
    pub fn connect_nodes(&self, from: NodeId, to: NodeId, channel: mpsc::UnboundedSender<crate::message::Message>) {
        self.channels.insert((from, to), channel);
    }
    
    pub async fn send_message(&self, from: NodeId, to: NodeId, message: crate::message::Message) -> crate::Result<()> {
        // Check for partition
        if self.is_partitioned(from, to) {
            return Err(crate::Error::NetworkPartition);
        }
        
        // Find nodes to determine latency
        let from_node = self.nodes.iter().find(|n| n.id == from);
        let to_node = self.nodes.iter().find(|n| n.id == to);
        
        if let (Some(from_node), Some(to_node)) = (from_node, to_node) {
            // Simulate latency based on node levels
            let level = from_node.level.max(to_node.level);
            self.latency.simulate_for_level(level).await;
        }
        
        // Send message
        if let Some(channel) = self.channels.get(&(from, to)) {
            channel.send(message)
                .map_err(|_| crate::Error::Other(anyhow::anyhow!("Channel closed")))?;
        }
        
        Ok(())
    }
    
    pub fn partition(&self, node1: NodeId, node2: NodeId) {
        self.partition_map.insert((node1, node2), true);
        self.partition_map.insert((node2, node1), true);
    }
    
    pub fn heal_partition(&self, node1: NodeId, node2: NodeId) {
        self.partition_map.remove(&(node1, node2));
        self.partition_map.remove(&(node2, node1));
    }
    
    fn is_partitioned(&self, from: NodeId, to: NodeId) -> bool {
        self.partition_map.get(&(from, to)).map(|v| *v).unwrap_or(false)
    }
}

pub async fn create_test_cluster(
    num_local: usize,
    num_regional: usize,
    num_global: usize,
) -> (Vec<Arc<RhcNode>>, NetworkSimulator) {
    let mut nodes = Vec::new();
    let mut network = NetworkSimulator::new(LatencySimulator::default());
    
    // Create global arbitrators
    for _ in 0..num_global {
        let node = Arc::new(RhcNode::new(
            NodeRole::GlobalArbitrator,
            3,
            Arc::new(InMemoryStorage::new()),
            None,
        ));
        nodes.push(node.clone());
        network.add_node(node);
    }
    
    // Create regional coordinators
    for i in 0..num_regional {
        let node = Arc::new(RhcNode::new(
            NodeRole::RegionalCoordinator,
            2,
            Arc::new(InMemoryStorage::new()),
            None, // DAG connections added later
        ));
        nodes.push(node.clone());
        network.add_node(node);
    }
    
    // Create local leaders
    for i in 0..num_local {
        let node = Arc::new(RhcNode::new(
            NodeRole::LocalLeader,
            1,
            Arc::new(InMemoryStorage::new()),
            None, // DAG connections added later
        ));
        nodes.push(node.clone());
        network.add_node(node);
    }
    
    // Create DAG topology
    // Global arbitrators connect to all regional coordinators
    for i in 0..num_global {
        for j in num_global..(num_global + num_regional) {
            nodes[i].add_peer(nodes[j].id, nodes[j].level);
            nodes[j].add_peer(nodes[i].id, nodes[i].level);
        }
    }
    
    // Regional coordinators connect to local leaders
    for i in num_global..(num_global + num_regional) {
        for j in (num_global + num_regional)..nodes.len() {
            nodes[i].add_peer(nodes[j].id, nodes[j].level);
            nodes[j].add_peer(nodes[i].id, nodes[i].level);
        }
    }
    
    // Local leaders can connect to each other (optional for CRDT merging)
    let local_start = num_global + num_regional;
    for i in local_start..nodes.len() {
        for j in (i+1)..nodes.len() {
            nodes[i].add_peer(nodes[j].id, nodes[j].level);
            nodes[j].add_peer(nodes[i].id, nodes[i].level);
        }
    }
    
    // Connect each node's message channel to the NetworkSimulator
    for node in &nodes {
        let node_id = node.id;
        let network_clone = Arc::new(network.clone());
        let nodes_clone = nodes.clone();
        
        // Get the node's message receiver and process messages through NetworkSimulator
        if let Some(mut rx) = node.message_rx.lock().take() {
            tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    // Route message through NetworkSimulator to all peers
                    for target_node in &nodes_clone {
                        if target_node.id != node_id {
                            if let Err(e) = network_clone.send_message(node_id, target_node.id, message.clone()).await {
                                println!("Network send failed: {}", e);
                            } else {
                                // Send to target node's internal handler
                                if let Err(e) = target_node.handle_message(message.clone()).await {
                                    println!("Message handling failed: {}", e);
                                }
                            }
                        }
                    }
                }
            });
        }
    }
    
    // Set up basic message channels (for NetworkSimulator internal use)
    for i in 0..nodes.len() {
        for j in 0..nodes.len() {
            if i != j {
                let (tx, _rx) = mpsc::unbounded_channel();
                network.connect_nodes(nodes[i].id, nodes[j].id, tx);
            }
        }
    }
    
    (nodes, network)
}

#[derive(Debug)]
pub struct LatencyMeasurement {
    pub operation: String,
    pub start_time: std::time::Instant,
    pub end_time: std::time::Instant,
    pub latency_us: u64,
}

impl LatencyMeasurement {
    pub fn start(operation: &str) -> Self {
        Self {
            operation: operation.to_string(),
            start_time: std::time::Instant::now(),
            end_time: std::time::Instant::now(),
            latency_us: 0,
        }
    }
    
    pub fn stop(&mut self) {
        self.end_time = std::time::Instant::now();
        self.latency_us = self.end_time.duration_since(self.start_time).as_micros() as u64;
    }
    
    pub fn assert_microseconds(&self, max_us: u64) {
        assert!(
            self.latency_us <= max_us,
            "{} took {}μs, expected <= {}μs",
            self.operation, self.latency_us, max_us
        );
    }
    
    pub fn assert_milliseconds(&self, max_ms: u64) {
        let ms = self.latency_us / 1000;
        assert!(
            ms <= max_ms,
            "{} took {}ms, expected <= {}ms",
            self.operation, ms, max_ms
        );
    }
}
use crate::{
    consensus::BftConsensus,
    crypto::Ed25519KeyPair,
    lease::{Domain, LeaseManager, LeaseProof},
    message::{Heartbeat, LoadInfo, Message, Operation, SyncBatch},
    storage::Storage,
    time::{HybridClock, HybridTimestamp},
    NodeId, Result,
};
use std::collections::HashMap;
use std::path::PathBuf;
use chrono::Duration;
use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    LocalLeader,
    RegionalCoordinator,
    GlobalArbitrator,
    Hybrid,
}

#[derive(Debug)]
pub struct RhcNode {
    pub id: NodeId,
    pub role: NodeRole,
    pub level: u8,
    clock: Arc<HybridClock>,
    lease_manager: Arc<LeaseManager>,
    consensus: Option<Arc<BftConsensus>>,
    storage: Arc<dyn Storage>,
    keypair: Ed25519KeyPair,
    peers: Arc<DashMap<NodeId, NodeInfo>>, // DAG peers at all levels
    children: Arc<DashMap<NodeId, NodeInfo>>, // Direct children for lease delegation
    path_leases: Arc<DashMap<PathBuf, PathLeaseInfo>>, // Dynamic path-scoped leases
    access_patterns: Arc<DashMap<PathBuf, HashMap<NodeId, AccessPattern>>>, // Usage tracking
    pending_operations: Arc<RwLock<Vec<Operation>>>,
    message_tx: mpsc::UnboundedSender<Message>,
    pub message_rx: Arc<parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Message>>>>,
}

#[derive(Debug, Clone)]
struct NodeInfo {
    id: NodeId,
    last_heartbeat: crate::time::HybridTimestamp,
    load: LoadInfo,
}

#[derive(Debug, Clone)]
struct AccessPattern {
    path: PathBuf,
    node: NodeId,
    read_ops_per_sec: f64,
    write_ops_per_sec: f64,
    last_access: HybridTimestamp,
    access_trend: AccessTrend,
}

#[derive(Debug, Clone, PartialEq)]
enum AccessTrend {
    Increasing,
    Decreasing,
    Stable,
}

#[derive(Debug, Clone)]
struct PathLeaseInfo {
    path: PathBuf,
    current_holder: NodeId,
    lease_proof: LeaseProof,
    access_patterns: HashMap<NodeId, AccessPattern>,
    last_migration_time: HybridTimestamp,
}

impl RhcNode {
    pub fn new(
        role: NodeRole,
        level: u8,
        storage: Arc<dyn Storage>,
        _parent: Option<NodeId>, // Deprecated - use add_peer instead
    ) -> Self {
        let id = NodeId::new();
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        
        let consensus = match role {
            NodeRole::GlobalArbitrator | NodeRole::Hybrid => {
                Some(Arc::new(BftConsensus::new(id, 3, 4, message_tx.clone())))
            }
            _ => None,
        };
        
        Self {
            id,
            role,
            level,
            clock: Arc::new(HybridClock::new()),
            lease_manager: Arc::new(LeaseManager::new(id)),
            consensus,
            storage,
            keypair: Ed25519KeyPair::generate(),
            peers: Arc::new(DashMap::new()),
            children: Arc::new(DashMap::new()),
            path_leases: Arc::new(DashMap::new()),
            access_patterns: Arc::new(DashMap::new()),
            pending_operations: Arc::new(RwLock::new(Vec::new())),
            message_tx,
            message_rx: Arc::new(parking_lot::Mutex::new(Some(message_rx))),
        }
    }
    
    pub async fn start(&self) -> Result<()> {
        // Start message processing loop
        let node_clone = self.clone_for_message_processing();
        if let Some(mut rx) = self.message_rx.lock().take() {
            tokio::spawn(async move {
                while let Some(message) = rx.recv().await {
                    if let Err(e) = node_clone.handle_message(message).await {
                        println!("Error processing message: {}", e);
                    }
                }
            });
        }
        
        // Start periodic operations flushing
        let flush_clone = self.clone_for_message_processing();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(10)); // Flush every 10ms
            loop {
                interval.tick().await;
                if let Err(e) = flush_clone.flush_operations().await {
                    println!("Error flushing operations: {}", e);
                }
            }
        });
        
        // Start heartbeat task
        let node_id = self.id;
        let clock = self.clock.clone();
        let message_tx = self.message_tx.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                
                let heartbeat = Heartbeat {
                    node_id,
                    epoch: 0,
                    active_leases: vec![],
                    load: LoadInfo {
                        cpu_usage: 0.0,
                        memory_usage: 0.0,
                        pending_operations: 0,
                        latency_ms: 0.0,
                    },
                    timestamp: clock.now(),
                };
                
                let _ = message_tx.send(Message::Heartbeat(heartbeat));
            }
        });
        
        Ok(())
    }
    
    pub async fn handle_message(&self, message: Message) -> Result<()> {
        match message {
            Message::LeaseRequest(req) => {
                self.handle_lease_request(req).await?;
            }
            Message::Propose(proposal) => {
                if let Some(consensus) = &self.consensus {
                    consensus.handle_proposal(proposal).await?;
                }
            }
            Message::ThresholdShare(share) => {
                if let Some(consensus) = &self.consensus {
                    consensus.handle_share(share).await?;
                }
            }
            Message::Commit(commit) => {
                if let Some(consensus) = &self.consensus {
                    consensus.handle_commit(commit).await?;
                }
            }
            Message::Heartbeat(hb) => {
                self.handle_heartbeat(hb).await?;
            }
            Message::SyncBatch(batch) => {
                self.handle_sync_batch(batch).await?;
            }
            _ => {}
        }
        Ok(())
    }
    
    async fn handle_lease_request(&self, req: crate::message::LeaseRequest) -> Result<()> {
        // Only handle if we're the right level
        if req.domain.level != self.level {
            return Ok(());
        }
        
        let proof = self.lease_manager.request_lease(
            &req.domain,
            req.duration,
            req.parent_lease_proof,
        ).await?;
        
        let grant = crate::message::LeaseGrant {
            request_id: req.request_id,
            lease_proof: proof,
            timestamp: self.clock.now(),
        };
        
        self.message_tx.send(Message::LeaseGrant(grant))
            .map_err(|_| crate::Error::Other(anyhow::anyhow!("Failed to send message")))?;
        
        Ok(())
    }
    
    async fn handle_heartbeat(&self, hb: Heartbeat) -> Result<()> {
        let info = NodeInfo {
            id: hb.node_id,
            last_heartbeat: hb.timestamp,
            load: hb.load,
        };
        
        self.children.insert(hb.node_id, info);
        Ok(())
    }
    
    async fn handle_sync_batch(&self, batch: SyncBatch) -> Result<()> {
        println!("Node {:?} received sync batch with {} operations from {:?}", 
                self.id, batch.operations.len(), batch.source);
        
        // Verify lease for each operation
        for op in &batch.operations {
            if let Err(e) = self.lease_manager.verify_lease(&op.lease_proof) {
                println!("Lease verification failed for operation {}: {}", op.id, e);
                // Continue with other operations instead of failing entirely
                continue;
            }
        }
        
        // Apply operations to storage
        for op in batch.operations {
            if let Err(e) = self.storage.apply_operation(&op).await {
                println!("Failed to apply operation {}: {}", op.id, e);
            } else {
                println!("Node {:?} applied operation {} successfully", self.id, op.id);
            }
        }
        
        Ok(())
    }
    
    pub async fn write(&self, key: &str, value: Vec<u8>, lease_proof: LeaseProof) -> Result<()> {
        let path = PathBuf::from(key);
        
        // Record access pattern for dynamic migration
        self.record_access_pattern(&path, true).await;
        
        // Check if we should migrate the lease to this node
        if let Some(migration_needed) = self.should_migrate_lease(&path).await {
            if migration_needed {
                self.request_lease_migration(&path).await?;
            }
        }
        
        // Verify we hold the lease (after potential migration)
        self.lease_manager.verify_lease(&lease_proof)?;
        
        let op = Operation {
            id: Uuid::new_v4(),
            op_type: crate::message::OperationType::Write,
            data: bincode::serialize(&(key.to_string(), value))?,
            lease_proof,
            timestamp: self.clock.now(),
        };
        
        // Apply locally
        self.storage.apply_operation(&op).await?;
        
        // Queue for replication
        self.pending_operations.write().push(op);
        
        // Always flush immediately for testing (in production, use batching)
        self.flush_operations().await?;
        
        Ok(())
    }
    
    async fn record_access_pattern(&self, path: &PathBuf, is_write: bool) {
        let now = self.clock.now();
        let mut patterns = self.access_patterns.entry(path.clone()).or_insert_with(HashMap::new);
        
        let pattern = patterns.entry(self.id).or_insert_with(|| AccessPattern {
            path: path.clone(),
            node: self.id,
            read_ops_per_sec: 0.0,
            write_ops_per_sec: 0.0,
            last_access: now,
            access_trend: AccessTrend::Stable,
        });
        
        // Update access pattern
        if is_write {
            pattern.write_ops_per_sec += 1.0; // Simplified - should use sliding window
        } else {
            pattern.read_ops_per_sec += 1.0;
        }
        pattern.last_access = now;
        
        // Simple trend detection
        let total_ops = pattern.read_ops_per_sec + pattern.write_ops_per_sec;
        pattern.access_trend = if total_ops > 10.0 {
            AccessTrend::Increasing
        } else if total_ops < 1.0 {
            AccessTrend::Decreasing
        } else {
            AccessTrend::Stable
        };
    }
    
    async fn should_migrate_lease(&self, path: &PathBuf) -> Option<bool> {
        // Check if we already hold the lease for this path
        if let Some(lease_info) = self.path_leases.get(path) {
            if lease_info.current_holder == self.id {
                return Some(false); // We already hold it
            }
            
            // Calculate migration benefit
            let migration_score = self.calculate_migration_score(path, &lease_info).await;
            Some(migration_score > 5.0) // Threshold for migration
        } else {
            // New path - we should create a lease
            Some(true)
        }
    }
    
    async fn calculate_migration_score(&self, path: &PathBuf, lease_info: &PathLeaseInfo) -> f64 {
        let patterns = self.access_patterns.get(path);
        if let Some(patterns) = patterns {
            let current_holder_activity = patterns.get(&lease_info.current_holder)
                .map(|p| p.read_ops_per_sec + p.write_ops_per_sec)
                .unwrap_or(0.0);
            
            let our_activity = patterns.get(&self.id)
                .map(|p| p.read_ops_per_sec + p.write_ops_per_sec)
                .unwrap_or(0.0);
            
            // Simple scoring: our activity vs current holder activity
            our_activity - current_holder_activity
        } else {
            0.0
        }
    }
    
    async fn request_lease_migration(&self, path: &PathBuf) -> Result<()> {
        println!("Node {:?} requesting lease migration for {:?}", self.id, path);
        
        // For now, just create a local lease (simplified)
        // In a full implementation, this would coordinate with other nodes
        let domain = Domain::new(path.to_string_lossy().to_string(), None, self.level);
        let lease_proof = self.lease_manager.request_lease(&domain, chrono::Duration::minutes(10), None).await?;
        
        let lease_info = PathLeaseInfo {
            path: path.clone(),
            current_holder: self.id,
            lease_proof,
            access_patterns: HashMap::new(),
            last_migration_time: self.clock.now(),
        };
        
        self.path_leases.insert(path.clone(), lease_info);
        Ok(())
    }
    
    pub fn add_peer(&self, peer_id: NodeId, level: u8) {
        let info = NodeInfo {
            id: peer_id,
            last_heartbeat: self.clock.now(),
            load: LoadInfo {
                cpu_usage: 0.0,
                memory_usage: 0.0,
                pending_operations: 0,
                latency_ms: 0.0,
            },
        };
        self.peers.insert(peer_id, info);
    }
    
    async fn flush_operations(&self) -> Result<()> {
        let ops: Vec<Operation> = {
            let mut pending = self.pending_operations.write();
            std::mem::take(&mut *pending)
        };
        
        if ops.is_empty() {
            return Ok(());
        }
        
        let domain = Domain::new("rhc_domain".to_string(), None, self.level);
        
        let batch = SyncBatch {
            source: self.id,
            domain,
            operations: ops,
            checkpoint: crate::message::Checkpoint {
                epoch: 0,
                state_hash: [0; 32], // TODO: Calculate actual hash
                operation_count: 0,
                signature: self.keypair.sign(&[0; 32]),
            },
            timestamp: self.clock.now(),
        };
        
        // Send to all DAG peers - multi-path propagation
        for _peer in self.peers.iter() {
            let batch_clone = batch.clone();
            self.message_tx.send(Message::SyncBatch(batch_clone))
                .map_err(|_| crate::Error::Other(anyhow::anyhow!("Failed to send message")))?;
        }
        
        Ok(())
    }
    
    pub async fn request_lease(
        &self,
        domain_name: &str,
        duration: Duration,
    ) -> Result<LeaseProof> {
        let domain = Domain::new(domain_name.to_string(), None, self.level);
        
        self.lease_manager.request_lease(&domain, duration, None).await
    }
    
    pub fn clone_for_message_processing(&self) -> Self {
        let (message_tx, _message_rx) = mpsc::unbounded_channel(); // New channel for clone
        Self {
            id: self.id,
            role: self.role,
            level: self.level,
            clock: self.clock.clone(),
            lease_manager: self.lease_manager.clone(),
            consensus: self.consensus.clone(),
            storage: self.storage.clone(),
            keypair: self.keypair.clone(),
            peers: self.peers.clone(),
            children: self.children.clone(),
            path_leases: self.path_leases.clone(),
            access_patterns: self.access_patterns.clone(),
            pending_operations: self.pending_operations.clone(),
            message_tx,
            message_rx: Arc::new(parking_lot::Mutex::new(None)), // No receiver for clones
        }
    }
    
    pub fn clone_for_task(&self) -> Self {
        let (message_tx, _message_rx) = mpsc::unbounded_channel();
        Self {
            id: self.id,
            role: self.role,
            level: self.level,
            clock: self.clock.clone(),
            lease_manager: self.lease_manager.clone(),
            consensus: self.consensus.clone(),
            storage: self.storage.clone(),
            keypair: self.keypair.clone(),
            peers: self.peers.clone(),
            children: self.children.clone(),
            path_leases: self.path_leases.clone(),
            access_patterns: self.access_patterns.clone(),
            pending_operations: self.pending_operations.clone(),
            message_tx,
            message_rx: Arc::new(parking_lot::Mutex::new(None)),
        }
    }
    
    pub fn clone(&self) -> Self {
        self.clone_for_task()
    }
    
    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }
    
    pub fn lease_manager(&self) -> &Arc<LeaseManager> {
        &self.lease_manager
    }
}
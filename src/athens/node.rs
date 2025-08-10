// Athens node - handles consensus for a single location

use crate::rhc::{
    hlc::{HLC, HLCTimestamp},
    leases::{Lease, LeaseManager, LeaseScope},
};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

/// Configuration for an Athens node
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Unique identifier for this node
    pub node_id: String,
    
    /// Geographic location (e.g., "perth", "london-main")
    pub location: String,
    
    /// Parent consensus group (if any)
    pub parent_cg: Option<String>,
    
    /// Storage class of this node
    pub storage_class: StorageClass,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StorageClass {
    NVMe,
    SSD,
    HDD,
    Hybrid,
}

/// An Athens consensus node
pub struct AthensNode {
    config: NodeConfig,
    hlc: Arc<HLC>,
    lease_manager: Arc<LeaseManager>,
    
    /// Network latencies to other nodes (for simulation)
    latencies: Arc<Mutex<HashMap<String, u64>>>, // node_id -> latency_ms
}

impl AthensNode {
    /// Create a new Athens node
    pub fn new(config: NodeConfig) -> Self {
        let hlc = Arc::new(HLC::new());
        let lease_manager = Arc::new(LeaseManager::new(
            config.node_id.clone(),
            hlc.clone(),
        ));

        Self {
            config,
            hlc,
            lease_manager,
            latencies: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set simulated network latency to another node
    pub fn set_latency(&self, to_node: String, latency_ms: u64) {
        let mut latencies = self.latencies.lock().unwrap();
        latencies.insert(to_node, latency_ms);
    }

    /// Get simulated network latency to another node
    pub fn get_latency(&self, to_node: &str) -> u64 {
        let latencies = self.latencies.lock().unwrap();
        latencies.get(to_node).copied().unwrap_or(0)
    }

    /// Request a lease for a path
    pub fn request_lease(
        &self,
        path: std::path::PathBuf,
        recursive: bool,
    ) -> Result<Lease, String> {
        let scope = if path.is_dir() {
            LeaseScope::Directory { path, recursive }
        } else {
            LeaseScope::File(path)
        };

        self.lease_manager
            .acquire_lease(scope, std::time::Duration::from_secs(30))
            .map_err(|e| format!("Failed to acquire lease: {:?}", e))
    }

    /// Check if we can write to a path
    pub fn can_write(&self, path: &std::path::PathBuf) -> bool {
        self.lease_manager.can_write(path)
    }

    /// Get current HLC timestamp
    pub fn now(&self) -> HLCTimestamp {
        self.hlc.now()
    }

    /// Simulate receiving a message from another node
    pub fn receive_message(&self, from: &str, msg: NetworkMessage) -> Result<(), String> {
        // Simulate network latency
        let latency_ms = self.get_latency(from);
        if latency_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(latency_ms));
        }

        match msg {
            NetworkMessage::LeaseRequest { path, timestamp } => {
                // Update our HLC with the remote timestamp
                self.hlc.update(timestamp)
                    .map_err(|e| format!("HLC error: {:?}", e))?;
                
                // TODO: Handle lease request consensus
                println!("{} received lease request for {:?} from {}", 
                    self.config.node_id, path, from);
            }
            NetworkMessage::LeaseTransfer { lease_id, new_holder } => {
                // TODO: Handle lease transfer
                println!("{} received lease transfer {:?} to {} from {}",
                    self.config.node_id, lease_id, new_holder, from);
            }
        }
        Ok(())
    }

    /// Get node info for debugging
    pub fn info(&self) -> String {
        format!(
            "Athens Node: {} @ {} ({})",
            self.config.node_id,
            self.config.location,
            match self.config.storage_class {
                StorageClass::NVMe => "NVMe",
                StorageClass::SSD => "SSD",
                StorageClass::HDD => "HDD",
                StorageClass::Hybrid => "Hybrid",
            }
        )
    }
}
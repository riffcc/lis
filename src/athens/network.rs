// Network simulation for Athens consensus

use crate::rhc::hlc::HLCTimestamp;
use crate::rhc::leases::LeaseId;
use std::path::PathBuf;

/// Messages that can be sent between Athens nodes
#[derive(Debug, Clone)]
pub enum NetworkMessage {
    /// Request a lease for a path
    LeaseRequest {
        path: PathBuf,
        timestamp: HLCTimestamp,
    },
    
    /// Transfer a lease to a new holder
    LeaseTransfer {
        lease_id: LeaseId,
        new_holder: String,
    },
}

/// Simulates network conditions between nodes
pub struct LatencySimulator {
    /// Base latencies between locations
    location_latencies: std::collections::HashMap<(String, String), u64>,
}

impl LatencySimulator {
    /// Create a realistic latency simulator
    pub fn new() -> Self {
        let mut location_latencies = std::collections::HashMap::new();
        
        // Perth <-> London: ~250ms
        location_latencies.insert(("perth".to_string(), "london".to_string()), 250);
        location_latencies.insert(("london".to_string(), "perth".to_string()), 250);
        
        // London sites: ~1-5ms
        location_latencies.insert(("london-main".to_string(), "london-mini".to_string()), 2);
        location_latencies.insert(("london-mini".to_string(), "london-main".to_string()), 2);
        location_latencies.insert(("london-main".to_string(), "london-secondary".to_string()), 5);
        location_latencies.insert(("london-secondary".to_string(), "london-main".to_string()), 5);
        location_latencies.insert(("london-mini".to_string(), "london-secondary".to_string()), 3);
        location_latencies.insert(("london-secondary".to_string(), "london-mini".to_string()), 3);
        
        // Same location: ~0.1ms
        location_latencies.insert(("perth".to_string(), "perth".to_string()), 0);
        location_latencies.insert(("london-main".to_string(), "london-main".to_string()), 0);
        
        Self { location_latencies }
    }

    /// Get latency between two locations
    pub fn get_latency(&self, from: &str, to: &str) -> u64 {
        if from == to {
            return 0;
        }

        // Check specific mapping
        if let Some(&latency) = self.location_latencies.get(&(from.to_string(), to.to_string())) {
            return latency;
        }

        // Check reverse mapping
        if let Some(&latency) = self.location_latencies.get(&(to.to_string(), from.to_string())) {
            return latency;
        }

        // Default: Check if cross-region
        let from_region = if from.starts_with("london") { "london" } else { from };
        let to_region = if to.starts_with("london") { "london" } else { to };
        
        if from_region != to_region {
            250 // Default cross-region latency
        } else {
            5 // Default same-region latency
        }
    }
}
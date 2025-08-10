// Lease manager - handles acquisition, renewal, and delegation

use super::{Lease, LeaseId, LeaseScope, LeaseState};
use crate::rhc::hlc::{HLC, HLCTimestamp};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Manages lease lifecycle for a node/CG
pub struct LeaseManager {
    /// HLC for consistent timestamps
    hlc: Arc<HLC>,
    
    /// Current lease state
    state: Arc<Mutex<LeaseState>>,
    
    /// Node/CG identifier
    node_id: String,
    
    /// Pre-committed approvals for delegation
    /// Maps (from_cg, path_pattern) -> approval
    delegations: Arc<Mutex<HashMap<(String, String), DelegationApproval>>>,
}

/// A pre-committed approval for lease delegation
#[derive(Debug, Clone)]
pub struct DelegationApproval {
    /// Pattern this approval covers (e.g., "/data/eu/*")
    pub path_pattern: String,
    
    /// CG that can use this approval
    pub approved_for: String,
    
    /// When this approval expires
    pub valid_until: HLCTimestamp,
    
    /// Cryptographic signature (placeholder)
    pub signature: Vec<u8>,
}

/// Result of a lease operation
pub type LeaseResult<T> = Result<T, LeaseError>;

/// Errors that can occur during lease operations
#[derive(Debug, Clone)]
pub enum LeaseError {
    /// Another lease conflicts with this request
    Conflict { existing: LeaseId },
    
    /// Lease has expired
    Expired { lease_id: LeaseId },
    
    /// Not authorized to perform operation
    Unauthorized,
    
    /// Lease not found
    NotFound { lease_id: LeaseId },
}

impl LeaseManager {
    /// Create a new lease manager
    pub fn new(node_id: String, hlc: Arc<HLC>) -> Self {
        Self {
            hlc,
            state: Arc::new(Mutex::new(LeaseState::new(node_id.clone()))),
            node_id,
            delegations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Request a new lease
    pub fn acquire_lease(
        &self,
        scope: LeaseScope,
        duration: Duration,
    ) -> LeaseResult<Lease> {
        let now = self.hlc.now();
        let mut state = self.state.lock().unwrap();

        // Check if we can acquire this lease
        if let Err(existing) = state.can_acquire_lease(&scope, now) {
            return Err(LeaseError::Conflict { existing: existing.id });
        }

        // Create new lease
        let lease = Lease::new(
            scope,
            self.node_id.clone(),
            now,
            duration,
        );

        state.add_lease(lease.clone());
        Ok(lease)
    }

    /// Renew an existing lease
    pub fn renew_lease(
        &self,
        lease_id: LeaseId,
        duration: Duration,
    ) -> LeaseResult<()> {
        let now = self.hlc.now();
        let mut state = self.state.lock().unwrap();

        // Find and update the lease
        if let Some(lease) = state.get_lease_mut(&lease_id) {
            if lease.holder != self.node_id {
                return Err(LeaseError::Unauthorized);
            }
            if lease.is_expired(now) {
                return Err(LeaseError::Expired { lease_id });
            }
            
            lease.renew(now, duration);
            Ok(())
        } else {
            Err(LeaseError::NotFound { lease_id })
        }
    }

    /// Release a lease early
    pub fn release_lease(&self, lease_id: LeaseId) -> LeaseResult<()> {
        let mut state = self.state.lock().unwrap();

        if let Some(lease) = state.get_lease(&lease_id) {
            if lease.holder != self.node_id {
                return Err(LeaseError::Unauthorized);
            }
            state.remove_lease(lease_id);
            Ok(())
        } else {
            Err(LeaseError::NotFound { lease_id })
        }
    }

    /// Add a pre-committed delegation approval
    pub fn add_delegation(
        &self,
        from_cg: String,
        path_pattern: String,
        approved_for: String,
        valid_for: Duration,
    ) {
        let now = self.hlc.now();
        let valid_until = HLCTimestamp::new(
            now.physical + valid_for.as_millis() as u64,
            0,
        );

        let approval = DelegationApproval {
            path_pattern: path_pattern.clone(),
            approved_for,
            valid_until,
            signature: vec![], // TODO: Implement actual signatures
        };

        let mut delegations = self.delegations.lock().unwrap();
        delegations.insert((from_cg, path_pattern), approval);
    }

    /// Get leases that need renewal soon
    pub fn leases_needing_renewal(&self, within: Duration) -> Vec<Lease> {
        let now = self.hlc.now();
        let state = self.state.lock().unwrap();
        
        state.expiring_soon(now, within)
            .into_iter()
            .filter(|lease| lease.holder == self.node_id)
            .cloned()
            .collect()
    }

    /// Clean up expired leases and delegations
    pub fn cleanup(&self) {
        let now = self.hlc.now();
        
        // Clean up leases
        {
            let mut state = self.state.lock().unwrap();
            state.cleanup_expired(now);
        }

        // Clean up delegations
        {
            let mut delegations = self.delegations.lock().unwrap();
            delegations.retain(|_, approval| approval.valid_until > now);
        }
    }

    /// Check if we hold a valid lease for writing to a path
    pub fn can_write(&self, path: &std::path::PathBuf) -> bool {
        let now = self.hlc.now();
        let state = self.state.lock().unwrap();
        
        if let Some(lease) = state.find_lease_for_path(path, now) {
            lease.holder == self.node_id && !lease.is_expired(now)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_lease_acquisition() {
        let hlc = Arc::new(HLC::new());
        let manager = LeaseManager::new("node1".to_string(), hlc);

        // Acquire a lease
        let scope = LeaseScope::File(PathBuf::from("/data/file.txt"));
        let lease = manager.acquire_lease(scope.clone(), Duration::from_secs(30)).unwrap();
        
        assert_eq!(lease.holder, "node1");
        assert!(manager.can_write(&PathBuf::from("/data/file.txt")));

        // Try to acquire conflicting lease
        let result = manager.acquire_lease(scope, Duration::from_secs(30));
        assert!(matches!(result, Err(LeaseError::Conflict { .. })));
    }

    #[test]
    fn test_lease_renewal() {
        let hlc = Arc::new(HLC::new());
        let manager = LeaseManager::new("node1".to_string(), hlc);

        // Acquire and renew a lease
        let scope = LeaseScope::File(PathBuf::from("/data/file.txt"));
        let lease = manager.acquire_lease(scope, Duration::from_secs(30)).unwrap();
        
        let result = manager.renew_lease(lease.id, Duration::from_secs(30));
        assert!(result.is_ok());
    }
}
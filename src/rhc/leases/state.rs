// Lease state tracking and validation

use super::{Lease, LeaseId, LeaseScope};
use crate::rhc::hlc::HLCTimestamp;
use std::collections::HashMap;

/// Current status of a lease
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseStatus {
    /// Lease is active and valid
    Active,
    /// Lease has expired
    Expired,
    /// Lease was revoked/released
    Revoked,
    /// Lease is being transferred
    Transferring,
}

/// Tracks the state of all leases in a node/CG
#[derive(Debug)]
pub struct LeaseState {
    /// All known leases by ID
    leases: HashMap<LeaseId, Lease>,
    
    /// Index of leases by path for fast lookup
    /// Maps path -> lease IDs that might cover it
    path_index: HashMap<std::path::PathBuf, Vec<LeaseId>>,
    
    /// Our node/CG identifier
    node_id: String,
}

impl LeaseState {
    /// Create a new lease state tracker
    pub fn new(node_id: String) -> Self {
        Self {
            leases: HashMap::new(),
            path_index: HashMap::new(),
            node_id,
        }
    }

    /// Get a lease by ID
    pub fn get_lease(&self, lease_id: &LeaseId) -> Option<&Lease> {
        self.leases.get(lease_id)
    }

    /// Get a mutable reference to a lease by ID
    pub fn get_lease_mut(&mut self, lease_id: &LeaseId) -> Option<&mut Lease> {
        self.leases.get_mut(lease_id)
    }

    /// Add a lease to the state
    pub fn add_lease(&mut self, lease: Lease) {
        let lease_id = lease.id;
        
        // Add to main storage
        self.leases.insert(lease_id, lease.clone());
        
        // Update path index (only for file/directory leases)
        if let Some(path) = lease.scope.path() {
            self.path_index
                .entry(path.clone())
                .or_insert_with(Vec::new)
                .push(lease_id);
        }
    }

    /// Remove a lease from the state
    pub fn remove_lease(&mut self, lease_id: LeaseId) -> Option<Lease> {
        if let Some(lease) = self.leases.remove(&lease_id) {
            // Remove from path index (only for file/directory leases)
            if let Some(path) = lease.scope.path() {
                if let Some(leases) = self.path_index.get_mut(path) {
                    leases.retain(|&id| id != lease_id);
                    if leases.is_empty() {
                        self.path_index.remove(path);
                    }
                }
            }
            Some(lease)
        } else {
            None
        }
    }

    /// Find the most specific active lease covering a path
    pub fn find_lease_for_path(
        &self,
        path: &std::path::PathBuf,
        now: HLCTimestamp,
    ) -> Option<&Lease> {
        let mut candidates = Vec::new();

        // Check all leases that might cover this path
        for (lease_path, lease_ids) in &self.path_index {
            // Quick check: if the lease path is a prefix of our path
            if path.starts_with(lease_path) || lease_path == path {
                for &lease_id in lease_ids {
                    if let Some(lease) = self.leases.get(&lease_id) {
                        if lease.scope.covers(path) && !lease.is_expired(now) {
                            candidates.push(lease);
                        }
                    }
                }
            }
        }

        // Return the most specific lease (most path components)
        candidates.into_iter()
            .max_by_key(|lease| {
                lease.scope.path()
                    .map(|p| p.components().count())
                    .unwrap_or(0)
            })
    }

    /// Get all active leases held by this node
    pub fn my_leases(&self, now: HLCTimestamp) -> Vec<&Lease> {
        self.leases
            .values()
            .filter(|lease| {
                lease.holder == self.node_id && !lease.is_expired(now)
            })
            .collect()
    }

    /// Get all leases expiring within the given duration
    pub fn expiring_soon(
        &self,
        now: HLCTimestamp,
        within: std::time::Duration,
    ) -> Vec<&Lease> {
        let within_ms = within.as_millis() as u64;
        let deadline = HLCTimestamp::new(now.physical + within_ms, 0);

        self.leases
            .values()
            .filter(|lease| {
                !lease.is_expired(now) && lease.expires_at <= deadline
            })
            .collect()
    }

    /// Clean up expired leases
    pub fn cleanup_expired(&mut self, now: HLCTimestamp) -> Vec<Lease> {
        let expired_ids: Vec<LeaseId> = self.leases
            .iter()
            .filter(|(_, lease)| lease.is_expired(now))
            .map(|(&id, _)| id)
            .collect();

        let mut removed = Vec::new();
        for lease_id in expired_ids {
            if let Some(lease) = self.remove_lease(lease_id) {
                removed.push(lease);
            }
        }
        removed
    }

    /// Check if we can acquire a lease for the given scope
    /// Returns conflicting lease if one exists
    pub fn can_acquire_lease(
        &self,
        scope: &LeaseScope,
        now: HLCTimestamp,
    ) -> Result<(), &Lease> {
        // Check if there's already a lease covering this scope (only for path-based leases)
        if let Some(path) = scope.path() {
            if let Some(existing) = self.find_lease_for_path(path, now) {
                // If the existing lease is less specific, we can override
                if scope.is_more_specific_than(&existing.scope) {
                    Ok(())
                } else {
                    Err(existing)
                }
            } else {
                Ok(())
            }
        } else {
            // Block leases don't conflict based on paths
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_lease_state_management() {
        let mut state = LeaseState::new("node1".to_string());
        let now = HLCTimestamp::new(1000, 0);

        // Add a lease
        let lease = Lease::new(
            LeaseScope::Directory {
                path: PathBuf::from("/data"),
                recursive: true,
            },
            "node1".to_string(),
            now,
            std::time::Duration::from_secs(30),
        );
        let lease_id = lease.id;
        state.add_lease(lease);

        // Find lease by path
        let found = state.find_lease_for_path(&PathBuf::from("/data/file.txt"), now);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, lease_id);

        // Remove lease
        let removed = state.remove_lease(lease_id);
        assert!(removed.is_some());
        
        // Should not find it anymore
        let not_found = state.find_lease_for_path(&PathBuf::from("/data/file.txt"), now);
        assert!(not_found.is_none());
    }

    #[test]
    fn test_most_specific_lease() {
        let mut state = LeaseState::new("node1".to_string());
        let now = HLCTimestamp::new(1000, 0);

        // Add general lease
        let general = Lease::new(
            LeaseScope::Directory {
                path: PathBuf::from("/data"),
                recursive: true,
            },
            "node1".to_string(),
            now,
            std::time::Duration::from_secs(30),
        );
        state.add_lease(general);

        // Add more specific lease
        let specific = Lease::new(
            LeaseScope::Directory {
                path: PathBuf::from("/data/eu/uk"),
                recursive: true,
            },
            "node2".to_string(),
            now,
            std::time::Duration::from_secs(30),
        );
        let specific_id = specific.id;
        state.add_lease(specific);

        // Should find the more specific lease
        let found = state.find_lease_for_path(&PathBuf::from("/data/eu/uk/london.txt"), now);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, specific_id);
    }
}
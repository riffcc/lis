// CRDT for managing lease state in consensus groups

use super::{CRDT, ActorId, LWWRegister};
use crate::rhc::hlc::HLCTimestamp;
use crate::rhc::leases::{LeaseId, LeaseScope};
use std::collections::HashMap;

/// CRDT for lease state that can handle concurrent updates
/// Uses LWW-Register for each lease entry
#[derive(Debug, Clone)]
pub struct LeaseStateCRDT {
    /// Map from lease scope to lease information
    leases: HashMap<String, LWWRegister<LeaseEntry>>,
    /// Node/CG ID that owns this CRDT
    actor: ActorId,
}

#[derive(Debug, Clone)]
pub struct LeaseEntry {
    pub holder: ActorId,
    pub lease_id: LeaseId,
    pub granted_at: HLCTimestamp,
    pub expires_at: HLCTimestamp,
    pub is_active: bool,
    pub fence_ts: Option<HLCTimestamp>,
}

impl LeaseStateCRDT {
    pub fn new(actor: ActorId) -> Self {
        Self {
            leases: HashMap::new(),
            actor,
        }
    }
    
    /// Grant a new lease
    pub fn grant_lease(
        &mut self,
        scope: &LeaseScope,
        holder: ActorId,
        granted_at: HLCTimestamp,
        expires_at: HLCTimestamp,
    ) -> LeaseId {
        let lease_id = LeaseId::new();
        let entry = LeaseEntry {
            holder,
            lease_id,
            granted_at,
            expires_at,
            is_active: true,
            fence_ts: None,
        };
        
        let scope_key = format!("{:?}", scope);
        self.leases
            .entry(scope_key)
            .or_insert_with(LWWRegister::new)
            .set(entry, granted_at);
            
        lease_id
    }
    
    /// Create a fence for lease migration
    pub fn fence_lease(&mut self, scope: &LeaseScope, fence_ts: HLCTimestamp) {
        let scope_key = format!("{:?}", scope);
        if let Some(reg) = self.leases.get_mut(&scope_key) {
            if let Some(mut entry) = reg.get_timestamped().cloned() {
                entry.value.fence_ts = Some(fence_ts);
                entry.value.is_active = false;
                reg.set(entry.value, fence_ts);
            }
        }
    }
    
    /// Check if a lease is valid at a given timestamp
    pub fn is_lease_valid(&self, scope: &LeaseScope, check_ts: HLCTimestamp) -> Option<LeaseInfo> {
        let scope_key = format!("{:?}", scope);
        self.leases.get(&scope_key).and_then(|reg| {
            reg.get_timestamped().and_then(|entry| {
                let lease = &entry.value;
                
                // Check if lease is fenced
                if let Some(fence_ts) = lease.fence_ts {
                    if check_ts > fence_ts {
                        return None; // Lease was fenced
                    }
                }
                
                // Check if lease is expired
                if check_ts > lease.expires_at {
                    return None; // Lease expired
                }
                
                // Lease is valid
                Some(LeaseInfo {
                    holder: lease.holder.clone(),
                    lease_id: lease.lease_id,
                    expires_at: lease.expires_at,
                })
            })
        })
    }
    
    /// Get all active leases
    pub fn active_leases(&self, now: HLCTimestamp) -> Vec<(String, LeaseInfo)> {
        self.leases
            .iter()
            .filter_map(|(scope, reg)| {
                reg.get_timestamped().and_then(|entry| {
                    let lease = &entry.value;
                    if lease.is_active && lease.expires_at > now {
                        Some((scope.clone(), LeaseInfo {
                            holder: lease.holder.clone(),
                            lease_id: lease.lease_id,
                            expires_at: lease.expires_at,
                        }))
                    } else {
                        None
                    }
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct LeaseInfo {
    pub holder: ActorId,
    pub lease_id: LeaseId,
    pub expires_at: HLCTimestamp,
}

impl CRDT for LeaseStateCRDT {
    fn merge(&mut self, other: &Self) {
        for (scope, other_reg) in &other.leases {
            self.leases
                .entry(scope.clone())
                .or_insert_with(LWWRegister::new)
                .merge(other_reg);
        }
    }
    
    fn happens_before(&self, _other: &Self) -> bool {
        // Lease state doesn't have a single timestamp
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    
    #[test]
    fn test_lease_grant_and_check() {
        let mut crdt = LeaseStateCRDT::new(ActorId::new("cg1"));
        let scope = LeaseScope::File(PathBuf::from("/data/file.txt"));
        let holder = ActorId::new("node1");
        
        let granted_at = HLCTimestamp::new(1000, 0);
        let expires_at = HLCTimestamp::new(2000, 0);
        
        let lease_id = crdt.grant_lease(&scope, holder.clone(), granted_at, expires_at);
        
        // Check at various times
        let check1 = HLCTimestamp::new(1500, 0); // During lease
        let info1 = crdt.is_lease_valid(&scope, check1);
        assert!(info1.is_some());
        assert_eq!(info1.unwrap().holder, holder);
        
        let check2 = HLCTimestamp::new(2500, 0); // After expiry
        let info2 = crdt.is_lease_valid(&scope, check2);
        assert!(info2.is_none());
    }
    
    #[test]
    fn test_lease_fence() {
        let mut crdt = LeaseStateCRDT::new(ActorId::new("cg1"));
        let scope = LeaseScope::File(PathBuf::from("/data/file.txt"));
        let holder = ActorId::new("node1");
        
        let granted_at = HLCTimestamp::new(1000, 0);
        let expires_at = HLCTimestamp::new(3000, 0);
        
        crdt.grant_lease(&scope, holder, granted_at, expires_at);
        
        // Fence at time 1500
        let fence_ts = HLCTimestamp::new(1500, 0);
        crdt.fence_lease(&scope, fence_ts);
        
        // Check before fence - should be valid
        let check1 = HLCTimestamp::new(1400, 0);
        assert!(crdt.is_lease_valid(&scope, check1).is_some());
        
        // Check after fence - should be invalid
        let check2 = HLCTimestamp::new(1600, 0);
        assert!(crdt.is_lease_valid(&scope, check2).is_none());
    }
    
    #[test]
    fn test_lease_crdt_merge() {
        let mut crdt1 = LeaseStateCRDT::new(ActorId::new("cg1"));
        let mut crdt2 = LeaseStateCRDT::new(ActorId::new("cg2"));
        
        let scope1 = LeaseScope::File(PathBuf::from("/data/file1.txt"));
        let scope2 = LeaseScope::File(PathBuf::from("/data/file2.txt"));
        
        // CG1 grants lease for file1
        crdt1.grant_lease(&scope1, ActorId::new("node1"), 
                         HLCTimestamp::new(1000, 0), 
                         HLCTimestamp::new(2000, 0));
        
        // CG2 grants lease for file2
        crdt2.grant_lease(&scope2, ActorId::new("node2"), 
                         HLCTimestamp::new(1100, 0), 
                         HLCTimestamp::new(2100, 0));
        
        // Merge CG2 into CG1
        crdt1.merge(&crdt2);
        
        // CG1 should now know about both leases
        let check_ts = HLCTimestamp::new(1500, 0);
        assert!(crdt1.is_lease_valid(&scope1, check_ts).is_some());
        assert!(crdt1.is_lease_valid(&scope2, check_ts).is_some());
    }
}
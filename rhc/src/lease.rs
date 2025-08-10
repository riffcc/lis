use crate::{crypto::Signature, error::Result, time::HybridTimestamp, NodeId};
use chrono::Duration;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Domain {
    pub id: Uuid,
    pub name: String,
    pub parent: Option<Uuid>,
    pub level: u8,
}

impl Domain {
    pub fn new(name: String, parent: Option<Uuid>, level: u8) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent,
            level,
        }
    }
    
    pub fn is_ancestor_of(&self, other: &Domain) -> bool {
        let mut current = other.parent;
        while let Some(parent_id) = current {
            if parent_id == self.id {
                return true;
            }
            // TODO: Look up parent domain
            current = None;
        }
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub id: Uuid,
    pub domain: Domain,
    pub holder: NodeId,
    pub start_time: HybridTimestamp,
    pub duration: Duration,
    pub parent_lease: Option<Box<LeaseProof>>,
    pub signature: Signature,
}

impl Lease {
    pub fn is_valid(&self, now: &HybridTimestamp) -> bool {
        let expiry = self.start_time.physical + self.duration;
        now.physical < expiry
    }
    
    pub fn expires_at(&self) -> HybridTimestamp {
        HybridTimestamp {
            physical: self.start_time.physical + self.duration,
            logical: self.start_time.logical,
        }
    }
    
    pub fn time_remaining(&self, now: &HybridTimestamp) -> Duration {
        let expiry = self.expires_at();
        if now.physical < expiry.physical {
            expiry.physical.signed_duration_since(now.physical)
        } else {
            Duration::zero()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseProof {
    pub lease: Lease,
    pub chain: Vec<Signature>,
}

impl LeaseProof {
    pub fn verify(&self) -> Result<()> {
        // TODO: Implement cryptographic verification
        Ok(())
    }
}

#[derive(Debug)]
pub struct LeaseManager {
    node_id: NodeId,
    leases: Arc<DashMap<Uuid, Lease>>,
    active_leases: Arc<DashMap<Uuid, Lease>>,
}

impl LeaseManager {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            leases: Arc::new(DashMap::new()),
            active_leases: Arc::new(DashMap::new()),
        }
    }
    
    pub async fn request_lease(
        &self,
        domain: &Domain,
        duration: Duration,
        parent_proof: Option<LeaseProof>,
    ) -> Result<LeaseProof> {
        // Check if domain is already leased
        if let Some(existing) = self.active_leases.get(&domain.id) {
            let now = crate::time::HybridClock::new().now();
            if existing.is_valid(&now) {
                return Err(crate::Error::LeaseConflict {
                    domain: domain.name.clone(),
                    holder: existing.holder,
                });
            }
        }
        
        // Create new lease
        let lease = Lease {
            id: Uuid::new_v4(),
            domain: domain.clone(),
            holder: self.node_id,
            start_time: crate::time::HybridClock::new().now(),
            duration,
            parent_lease: parent_proof.clone().map(Box::new),
            signature: Signature::default(), // TODO: Sign properly
        };
        
        // Store lease
        self.leases.insert(lease.id, lease.clone());
        self.active_leases.insert(domain.id, lease.clone());
        
        Ok(LeaseProof {
            lease,
            chain: vec![],
        })
    }
    
    pub fn verify_lease(&self, proof: &LeaseProof) -> Result<()> {
        proof.verify()?;
        
        let now = crate::time::HybridClock::new().now();
        if !proof.lease.is_valid(&now) {
            return Err(crate::Error::LeaseExpired {
                domain: proof.lease.domain.name.clone(),
            });
        }
        
        // Verify parent lease if exists
        if let Some(parent_proof) = &proof.lease.parent_lease {
            self.verify_lease(parent_proof)?;
            
            // Verify parent-child relationship
            if !parent_proof.lease.domain.is_ancestor_of(&proof.lease.domain) {
                return Err(crate::Error::InvalidLeaseProof);
            }
        }
        
        Ok(())
    }
    
    pub async fn renew_lease(&self, lease_id: &Uuid, duration: Duration) -> Result<LeaseProof> {
        let lease = self.leases.get(lease_id)
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("Lease not found")))?;
        
        let now = crate::time::HybridClock::new().now();
        if !lease.is_valid(&now) {
            return Err(crate::Error::LeaseExpired {
                domain: lease.domain.name.clone(),
            });
        }
        
        // Create renewed lease
        let renewed = Lease {
            id: Uuid::new_v4(),
            domain: lease.domain.clone(),
            holder: lease.holder,
            start_time: now,
            duration,
            parent_lease: lease.parent_lease.clone(),
            signature: Signature::default(), // TODO: Sign properly
        };
        
        self.leases.insert(renewed.id, renewed.clone());
        self.active_leases.insert(lease.domain.id, renewed.clone());
        
        Ok(LeaseProof {
            lease: renewed,
            chain: vec![],
        })
    }
    
    pub async fn revoke_lease(&self, lease_id: &Uuid) -> Result<()> {
        if let Some((_, lease)) = self.leases.remove(lease_id) {
            self.active_leases.remove(&lease.domain.id);
        }
        Ok(())
    }
    
    pub fn get_active_lease(&self, domain_id: &Uuid) -> Option<Lease> {
        self.active_leases.get(domain_id).map(|entry| entry.clone())
    }
}
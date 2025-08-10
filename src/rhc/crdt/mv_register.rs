// MV-Register (Multi-Value Register) CRDT implementation

use super::{CRDT, ActorId};
use crate::rhc::hlc::HLCTimestamp;
use std::collections::HashSet;

/// Multi-Value Register - stores multiple concurrent values
/// Returns all concurrent values that haven't been superseded
#[derive(Debug, Clone)]
pub struct MVRegister<T: Clone + Eq + std::hash::Hash> {
    /// Set of concurrent values with their vector clocks
    values: HashSet<VersionedValue<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionedValue<T> {
    pub value: T,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub actor: ActorId,
    pub timestamp: HLCTimestamp,
    /// Set of versions this supersedes
    pub supersedes: HashSet<(ActorId, HLCTimestamp)>,
}

// Manual Hash implementation to handle HashSet field
impl std::hash::Hash for Version {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.actor.hash(state);
        self.timestamp.hash(state);
        // Convert set to sorted vec for consistent hashing
        let mut supersedes_vec: Vec<_> = self.supersedes.iter().collect();
        supersedes_vec.sort();
        for item in supersedes_vec {
            item.hash(state);
        }
    }
}

impl<T: Clone + Eq + std::hash::Hash> MVRegister<T> {
    pub fn new() -> Self {
        Self {
            values: HashSet::new(),
        }
    }
    
    /// Set a value, superseding all current values
    pub fn set(&mut self, value: T, actor: ActorId, timestamp: HLCTimestamp) {
        // Collect all current versions to supersede
        let supersedes: HashSet<_> = self.values
            .iter()
            .map(|v| (v.version.actor.clone(), v.version.timestamp))
            .collect();
        
        // Remove superseded values
        self.values.clear();
        
        // Add new value
        self.values.insert(VersionedValue {
            value,
            version: Version {
                actor,
                timestamp,
                supersedes,
            },
        });
    }
    
    /// Get all concurrent values
    pub fn get(&self) -> Vec<&T> {
        self.values.iter().map(|v| &v.value).collect()
    }
    
    /// Get all values with their versions
    pub fn get_versioned(&self) -> Vec<&VersionedValue<T>> {
        self.values.iter().collect()
    }
    
    /// Check if register has any values
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl<T: Clone + Eq + std::hash::Hash> CRDT for MVRegister<T> {
    fn merge(&mut self, other: &Self) {
        // Collect all versions from both registers
        let all_values: HashSet<VersionedValue<T>> = 
            self.values.union(&other.values).cloned().collect();
        
        // Remove any values that are superseded by others
        let mut to_remove = HashSet::new();
        for v1 in &all_values {
            for v2 in &all_values {
                if v1 != v2 {
                    let v1_key = (v1.version.actor.clone(), v1.version.timestamp);
                    if v2.version.supersedes.contains(&v1_key) {
                        to_remove.insert(v1.clone());
                    }
                }
            }
        }
        
        self.values = all_values.difference(&to_remove).cloned().collect();
    }
    
    fn happens_before(&self, _other: &Self) -> bool {
        // MV-Register doesn't have a single happens-before relationship
        false
    }
}

impl<T: Clone + Eq + std::hash::Hash> Default for MVRegister<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mv_register_concurrent_values() {
        let mut reg1 = MVRegister::new();
        let mut reg2 = MVRegister::new();
        
        let actor1 = ActorId::new("node1");
        let actor2 = ActorId::new("node2");
        
        // Concurrent writes
        reg1.set("value1", actor1.clone(), HLCTimestamp::new(100, 0));
        reg2.set("value2", actor2.clone(), HLCTimestamp::new(100, 1));
        
        // Merge should preserve both values
        reg1.merge(&reg2);
        let values = reg1.get();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&&"value1"));
        assert!(values.contains(&&"value2"));
    }
    
    #[test]
    fn test_mv_register_superseding() {
        let mut reg = MVRegister::new();
        let actor = ActorId::new("node1");
        
        // Initial value
        reg.set("v1", actor.clone(), HLCTimestamp::new(100, 0));
        assert_eq!(reg.get().len(), 1);
        
        // New value supersedes old
        reg.set("v2", actor.clone(), HLCTimestamp::new(200, 0));
        let values = reg.get();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], &"v2");
    }
}
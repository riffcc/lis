// OR-Set (Observed-Remove Set) CRDT implementation

use super::{CRDT, ActorId};
use crate::rhc::hlc::HLCTimestamp;
use std::collections::{HashMap, HashSet};

/// OR-Set CRDT - supports concurrent add/remove operations
/// Each element has a set of unique tags (actor_id, timestamp) 
/// An element is in the set if it has at least one tag
#[derive(Debug, Clone)]
pub struct ORSet<T: Clone + Eq + std::hash::Hash> {
    /// Map from element to set of tags (actor_id, timestamp)
    elements: HashMap<T, HashSet<(ActorId, HLCTimestamp)>>,
}

impl<T: Clone + Eq + std::hash::Hash> ORSet<T> {
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
        }
    }
    
    /// Add an element to the set
    pub fn add(&mut self, element: T, actor: ActorId, timestamp: HLCTimestamp) {
        self.elements
            .entry(element)
            .or_insert_with(HashSet::new)
            .insert((actor, timestamp));
    }
    
    /// Remove an element from the set
    /// Only removes tags that are currently visible
    pub fn remove(&mut self, element: &T) {
        self.elements.remove(element);
    }
    
    /// Check if an element is in the set
    pub fn contains(&self, element: &T) -> bool {
        self.elements.contains_key(element)
    }
    
    /// Get all elements in the set
    pub fn elements(&self) -> Vec<&T> {
        self.elements.keys().collect()
    }
    
    /// Get the tags for an element
    pub fn tags(&self, element: &T) -> Option<&HashSet<(ActorId, HLCTimestamp)>> {
        self.elements.get(element)
    }
}

impl<T: Clone + Eq + std::hash::Hash> CRDT for ORSet<T> {
    fn merge(&mut self, other: &Self) {
        for (element, tags) in &other.elements {
            self.elements
                .entry(element.clone())
                .or_insert_with(HashSet::new)
                .extend(tags.iter().cloned());
        }
    }
    
    fn happens_before(&self, _other: &Self) -> bool {
        // OR-Set doesn't have a single timestamp, so this is element-specific
        false
    }
}

impl<T: Clone + Eq + std::hash::Hash> Default for ORSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_or_set_add_remove() {
        let mut set = ORSet::new();
        let actor1 = ActorId::new("node1");
        let actor2 = ActorId::new("node2");
        
        let ts1 = HLCTimestamp::new(100, 0);
        let ts2 = HLCTimestamp::new(200, 0);
        
        set.add("item1", actor1.clone(), ts1);
        assert!(set.contains(&"item1"));
        
        set.add("item1", actor2.clone(), ts2);
        assert_eq!(set.tags(&"item1").unwrap().len(), 2);
        
        set.remove(&"item1");
        assert!(!set.contains(&"item1"));
    }
    
    #[test]
    fn test_or_set_merge() {
        let mut set1 = ORSet::new();
        let mut set2 = ORSet::new();
        
        let actor1 = ActorId::new("node1");
        let actor2 = ActorId::new("node2");
        
        let ts1 = HLCTimestamp::new(100, 0);
        let ts2 = HLCTimestamp::new(200, 0);
        
        set1.add("item1", actor1.clone(), ts1);
        set2.add("item2", actor2.clone(), ts2);
        
        set1.merge(&set2);
        
        assert!(set1.contains(&"item1"));
        assert!(set1.contains(&"item2"));
        assert_eq!(set1.elements().len(), 2);
    }
}
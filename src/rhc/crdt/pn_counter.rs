// PN-Counter (Positive-Negative Counter) CRDT implementation

use super::{CRDT, ActorId};
use std::collections::HashMap;

/// PN-Counter - supports concurrent increment/decrement operations
/// Maintains separate positive and negative counters per actor
#[derive(Debug, Clone)]
pub struct PNCounter {
    /// Positive counts per actor
    positive: HashMap<ActorId, u64>,
    /// Negative counts per actor
    negative: HashMap<ActorId, u64>,
}

impl PNCounter {
    pub fn new() -> Self {
        Self {
            positive: HashMap::new(),
            negative: HashMap::new(),
        }
    }
    
    /// Increment the counter
    pub fn increment(&mut self, actor: ActorId, amount: u64) {
        *self.positive.entry(actor).or_insert(0) += amount;
    }
    
    /// Decrement the counter
    pub fn decrement(&mut self, actor: ActorId, amount: u64) {
        *self.negative.entry(actor).or_insert(0) += amount;
    }
    
    /// Get the current value of the counter
    pub fn value(&self) -> i64 {
        let pos_sum: u64 = self.positive.values().sum();
        let neg_sum: u64 = self.negative.values().sum();
        pos_sum as i64 - neg_sum as i64
    }
    
    /// Get the positive count for an actor
    pub fn positive_count(&self, actor: &ActorId) -> u64 {
        self.positive.get(actor).copied().unwrap_or(0)
    }
    
    /// Get the negative count for an actor
    pub fn negative_count(&self, actor: &ActorId) -> u64 {
        self.negative.get(actor).copied().unwrap_or(0)
    }
}

impl CRDT for PNCounter {
    fn merge(&mut self, other: &Self) {
        // For each actor, take the maximum of positive counts
        for (actor, &count) in &other.positive {
            let entry = self.positive.entry(actor.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
        
        // For each actor, take the maximum of negative counts
        for (actor, &count) in &other.negative {
            let entry = self.negative.entry(actor.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
    
    fn happens_before(&self, _other: &Self) -> bool {
        // PN-Counter doesn't have a happens-before relationship
        false
    }
}

impl Default for PNCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pn_counter_basic() {
        let mut counter = PNCounter::new();
        let actor1 = ActorId::new("node1");
        let actor2 = ActorId::new("node2");
        
        counter.increment(actor1.clone(), 5);
        counter.increment(actor2.clone(), 3);
        assert_eq!(counter.value(), 8);
        
        counter.decrement(actor1.clone(), 2);
        assert_eq!(counter.value(), 6);
    }
    
    #[test]
    fn test_pn_counter_merge() {
        let mut counter1 = PNCounter::new();
        let mut counter2 = PNCounter::new();
        
        let actor1 = ActorId::new("node1");
        let actor2 = ActorId::new("node2");
        
        counter1.increment(actor1.clone(), 5);
        counter1.decrement(actor1.clone(), 2);
        
        counter2.increment(actor2.clone(), 3);
        counter2.increment(actor1.clone(), 2); // Less than counter1
        
        counter1.merge(&counter2);
        
        // Should take max of each actor's counts
        assert_eq!(counter1.positive_count(&actor1), 5); // max(5, 2)
        assert_eq!(counter1.positive_count(&actor2), 3);
        assert_eq!(counter1.negative_count(&actor1), 2);
        assert_eq!(counter1.value(), 6); // (5 + 3) - 2
    }
}
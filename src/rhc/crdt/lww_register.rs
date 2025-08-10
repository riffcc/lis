// LWW-Register (Last-Write-Wins Register) CRDT implementation

use super::{CRDT, TimestampedValue};
use crate::rhc::hlc::HLCTimestamp;

/// Last-Write-Wins Register - stores a single value with timestamp
/// Concurrent writes are resolved by timestamp (last writer wins)
#[derive(Debug, Clone)]
pub struct LWWRegister<T: Clone> {
    value: Option<TimestampedValue<T>>,
}

impl<T: Clone> LWWRegister<T> {
    pub fn new() -> Self {
        Self { value: None }
    }
    
    /// Set the value with a timestamp
    pub fn set(&mut self, value: T, timestamp: HLCTimestamp) {
        match &self.value {
            None => {
                self.value = Some(TimestampedValue::new(value, timestamp));
            }
            Some(current) => {
                // Only update if new timestamp is greater
                if timestamp > current.timestamp {
                    self.value = Some(TimestampedValue::new(value, timestamp));
                }
            }
        }
    }
    
    /// Get the current value
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref().map(|tv| &tv.value)
    }
    
    /// Get the current value with timestamp
    pub fn get_timestamped(&self) -> Option<&TimestampedValue<T>> {
        self.value.as_ref()
    }
    
    /// Get the timestamp of the current value
    pub fn timestamp(&self) -> Option<HLCTimestamp> {
        self.value.as_ref().map(|tv| tv.timestamp)
    }
}

impl<T: Clone> CRDT for LWWRegister<T> {
    fn merge(&mut self, other: &Self) {
        match (&self.value, &other.value) {
            (None, None) => {}
            (Some(_), None) => {}
            (None, Some(other_val)) => {
                self.value = Some(other_val.clone());
            }
            (Some(self_val), Some(other_val)) => {
                // Keep the one with the later timestamp
                if other_val.timestamp > self_val.timestamp {
                    self.value = Some(other_val.clone());
                }
            }
        }
    }
    
    fn happens_before(&self, other: &Self) -> bool {
        match (&self.value, &other.value) {
            (Some(self_val), Some(other_val)) => self_val.timestamp < other_val.timestamp,
            (None, Some(_)) => true,
            _ => false,
        }
    }
}

impl<T: Clone> Default for LWWRegister<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_lww_register_basic() {
        let mut reg = LWWRegister::new();
        
        let ts1 = HLCTimestamp::new(100, 0);
        let ts2 = HLCTimestamp::new(200, 0);
        
        reg.set("value1", ts1);
        assert_eq!(reg.get(), Some(&"value1"));
        
        reg.set("value2", ts2);
        assert_eq!(reg.get(), Some(&"value2"));
        
        // Older timestamp should not overwrite
        let ts0 = HLCTimestamp::new(50, 0);
        reg.set("value0", ts0);
        assert_eq!(reg.get(), Some(&"value2"));
    }
    
    #[test]
    fn test_lww_register_merge() {
        let mut reg1 = LWWRegister::new();
        let mut reg2 = LWWRegister::new();
        
        let ts1 = HLCTimestamp::new(100, 0);
        let ts2 = HLCTimestamp::new(200, 0);
        
        reg1.set("value1", ts1);
        reg2.set("value2", ts2);
        
        reg1.merge(&reg2);
        assert_eq!(reg1.get(), Some(&"value2"));
        
        // Merge is commutative
        let mut reg3 = LWWRegister::new();
        let mut reg4 = LWWRegister::new();
        reg3.set("value1", ts1);
        reg4.set("value2", ts2);
        
        reg4.merge(&reg3);
        assert_eq!(reg4.get(), Some(&"value2"));
    }
}
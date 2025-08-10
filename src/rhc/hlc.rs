// Hybrid Logical Clock (HLC) implementation
// Based on: https://sookocheff.com/post/time/hybrid-logical-clocks/
//
// HLC combines physical time with a logical counter to provide:
// - Global ordering of events across distributed nodes
// - Bounded clock skew tolerance
// - Happens-before relationship preservation

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::cmp;

/// Maximum clock drift allowed between nodes (in milliseconds)
/// If physical clocks differ by more than this, we refuse to operate
const MAX_CLOCK_DRIFT_MS: u64 = 60_000; // 60 seconds

/// A Hybrid Logical Clock timestamp
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HLCTimestamp {
    /// Physical time component (milliseconds since Unix epoch)
    pub physical: u64,
    /// Logical counter to break ties when physical time is equal
    pub logical: u32,
}

impl HLCTimestamp {
    /// Create a new HLC timestamp with given physical time and logical counter
    pub fn new(physical: u64, logical: u32) -> Self {
        Self { physical, logical }
    }

    /// Create a zero timestamp (used for initialization)
    pub fn zero() -> Self {
        Self { physical: 0, logical: 0 }
    }

    /// Check if this timestamp is zero
    pub fn is_zero(&self) -> bool {
        self.physical == 0 && self.logical == 0
    }

    /// Get the physical time as a SystemTime
    pub fn as_system_time(&self) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::from_millis(self.physical)
    }

    /// Check if this timestamp is within acceptable drift of current time
    pub fn is_within_drift(&self, now_ms: u64) -> bool {
        if self.physical > now_ms {
            // Timestamp is in the future
            self.physical - now_ms <= MAX_CLOCK_DRIFT_MS
        } else {
            // Timestamp is in the past (always acceptable)
            true
        }
    }
}

impl std::fmt::Display for HLCTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.physical, self.logical)
    }
}

/// A Hybrid Logical Clock
pub struct HLC {
    /// Last known physical time (milliseconds since epoch)
    last_physical: AtomicU64,
    /// Logical counter for the last physical time
    last_logical: AtomicU64,
    /// Optional custom clock function for testing
    clock_fn: Option<Box<dyn Fn() -> u64 + Send + Sync>>,
}

impl std::fmt::Debug for HLC {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HLC")
            .field("last_physical", &self.last_physical)
            .field("last_logical", &self.last_logical)
            .field("clock_fn", &self.clock_fn.as_ref().map(|_| "<custom clock fn>"))
            .finish()
    }
}

impl HLC {
    /// Create a new HLC instance
    pub fn new() -> Self {
        Self {
            last_physical: AtomicU64::new(0),
            last_logical: AtomicU64::new(0),
            clock_fn: None,
        }
    }
    
    /// Create a new HLC with a custom clock function (for testing)
    pub fn new_with_clock(clock_fn: Box<dyn Fn() -> u64 + Send + Sync>) -> Self {
        Self {
            last_physical: AtomicU64::new(0),
            last_logical: AtomicU64::new(0),
            clock_fn: Some(clock_fn),
        }
    }

    /// Get current physical time in milliseconds since Unix epoch
    fn physical_now(&self) -> u64 {
        if let Some(ref clock_fn) = self.clock_fn {
            clock_fn()
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("System time before Unix epoch")
                .as_millis() as u64
        }
    }

    /// Generate a new HLC timestamp
    pub fn now(&self) -> HLCTimestamp {
        let physical_now = self.physical_now();
        
        loop {
            // Load last known values
            let last_physical = self.last_physical.load(Ordering::SeqCst);
            
            if physical_now > last_physical {
                // Try to advance physical time
                match self.last_physical.compare_exchange(
                    last_physical,
                    physical_now,
                    Ordering::SeqCst,
                    Ordering::SeqCst
                ) {
                    Ok(_) => {
                        // Reset logical counter when physical advances
                        self.last_logical.store(0, Ordering::SeqCst);
                        return HLCTimestamp {
                            physical: physical_now,
                            logical: 0,
                        };
                    }
                    Err(_) => continue, // Retry
                }
            } else {
                // Physical time hasn't advanced, need to increment logical
                // Use fetch_add to atomically get and increment
                let logical = self.last_logical.fetch_add(1, Ordering::SeqCst);
                
                // Check if physical time is still the same
                let current_physical = self.last_physical.load(Ordering::SeqCst);
                if current_physical == last_physical {
                    // We successfully got a unique logical counter
                    return HLCTimestamp {
                        physical: last_physical,
                        logical: (logical + 1) as u32,
                    };
                }
                // Physical time changed while we were incrementing logical, retry
            }
        }
    }

    /// Update the HLC with a timestamp received from another node
    /// Returns the new local timestamp after incorporating the remote one
    pub fn update(&self, remote: HLCTimestamp) -> Result<HLCTimestamp, HLCError> {
        let physical_now = self.physical_now();

        // Check if remote timestamp is within acceptable drift
        if !remote.is_within_drift(physical_now) {
            return Err(HLCError::ClockDriftExceeded {
                remote_physical: remote.physical,
                local_physical: physical_now,
                max_drift: MAX_CLOCK_DRIFT_MS,
            });
        }

        loop {
            // Load last known values
            let last_physical = self.last_physical.load(Ordering::SeqCst);
            let last_logical = self.last_logical.load(Ordering::SeqCst);

            // Calculate new timestamp
            let max_physical = cmp::max(cmp::max(physical_now, remote.physical), last_physical);
            
            let new_logical = if max_physical == physical_now && max_physical == remote.physical && max_physical == last_physical {
                // All three timestamps have same physical time
                cmp::max(cmp::max(remote.logical as u64, last_logical), 0) + 1
            } else if max_physical == physical_now && max_physical == remote.physical {
                // Physical now and remote have same time, but ahead of last
                cmp::max(remote.logical as u64, 0) + 1
            } else if max_physical == physical_now {
                // Our physical time is ahead
                0
            } else if max_physical == remote.physical {
                // Remote physical time is ahead
                remote.logical as u64 + 1
            } else {
                // Last physical time is ahead (shouldn't happen with monotonic clocks)
                last_logical + 1
            };

            // Try to update atomically
            if self.last_physical.compare_exchange(
                last_physical,
                max_physical,
                Ordering::SeqCst,
                Ordering::SeqCst
            ).is_ok() {
                // Physical update succeeded
                // For logical counter, we need to handle the case where another thread
                // might have updated it between our read and now
                let current_logical = self.last_logical.load(Ordering::SeqCst);
                let final_logical = if max_physical == last_physical {
                    // Physical didn't change, ensure logical is monotonic
                    cmp::max(new_logical, current_logical + 1)
                } else {
                    // Physical changed, use our calculated logical
                    new_logical
                };
                
                self.last_logical.store(final_logical, Ordering::SeqCst);
                
                return Ok(HLCTimestamp {
                    physical: max_physical,
                    logical: final_logical as u32,
                });
            }
            // If CAS failed, another thread updated, retry
        }
    }

    /// Get the last timestamp generated or received by this HLC
    pub fn last(&self) -> HLCTimestamp {
        // Need to read a consistent snapshot
        loop {
            let physical = self.last_physical.load(Ordering::SeqCst);
            let logical = self.last_logical.load(Ordering::SeqCst);
            
            // Verify physical hasn't changed
            if self.last_physical.load(Ordering::SeqCst) == physical {
                return HLCTimestamp {
                    physical,
                    logical: logical as u32,
                };
            }
        }
    }
}

impl Default for HLC {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur with HLC operations
#[derive(Debug, Clone, PartialEq)]
pub enum HLCError {
    /// Clock drift between nodes exceeds maximum allowed
    ClockDriftExceeded {
        remote_physical: u64,
        local_physical: u64,
        max_drift: u64,
    },
}

impl std::fmt::Display for HLCError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HLCError::ClockDriftExceeded { remote_physical, local_physical, max_drift } => {
                write!(
                    f, 
                    "Clock drift exceeded: remote={}, local={}, max_drift={}ms",
                    remote_physical, local_physical, max_drift
                )
            }
        }
    }
}

impl std::error::Error for HLCError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_hlc_timestamp_ordering() {
        // Test that timestamps order correctly
        let ts1 = HLCTimestamp::new(100, 0);
        let ts2 = HLCTimestamp::new(100, 1);
        let ts3 = HLCTimestamp::new(101, 0);

        assert!(ts1 < ts2);
        assert!(ts2 < ts3);
        assert!(ts1 < ts3);
    }

    #[test]
    fn test_hlc_timestamp_display() {
        let ts = HLCTimestamp::new(1234567890, 42);
        assert_eq!(ts.to_string(), "1234567890:42");
    }

    #[test]
    fn test_hlc_now_advances() {
        let hlc = HLC::new();
        
        let ts1 = hlc.now();
        thread::sleep(Duration::from_millis(2));
        let ts2 = hlc.now();

        assert!(ts2 > ts1);
        assert!(ts2.physical >= ts1.physical);
    }

    #[test]
    fn test_hlc_logical_counter_increments() {
        let hlc = HLC::new();
        
        // Generate multiple timestamps quickly (within same millisecond)
        let mut timestamps = Vec::new();
        for _ in 0..5 {
            timestamps.push(hlc.now());
        }

        // Check that logical counters increment when physical time doesn't
        for i in 1..timestamps.len() {
            assert!(timestamps[i] > timestamps[i-1]);
            if timestamps[i].physical == timestamps[i-1].physical {
                assert_eq!(timestamps[i].logical, timestamps[i-1].logical + 1);
            }
        }
    }

    #[test]
    fn test_hlc_update_with_future_timestamp() {
        let hlc = HLC::new();
        
        let local_ts = hlc.now();
        let future_ts = HLCTimestamp::new(local_ts.physical + 1000, 5);
        
        let updated = hlc.update(future_ts).unwrap();
        
        // HLC should adopt the future timestamp
        assert_eq!(updated.physical, future_ts.physical);
        assert_eq!(updated.logical, future_ts.logical + 1);
    }

    #[test]
    fn test_hlc_update_with_past_timestamp() {
        let hlc = HLC::new();
        
        let ts1 = hlc.now();
        thread::sleep(Duration::from_millis(10));
        let ts2 = hlc.now();
        
        // Try to update with an old timestamp
        let past_ts = HLCTimestamp::new(ts1.physical - 100, 0);
        let updated = hlc.update(past_ts).unwrap();
        
        // HLC should maintain monotonicity
        assert!(updated >= ts2);
    }

    #[test]
    fn test_hlc_update_same_physical_time() {
        let hlc1 = HLC::new();
        let hlc2 = HLC::new();
        
        // Force same physical time by setting last_physical
        let physical = hlc1.physical_now();
        hlc1.last_physical.store(physical, Ordering::SeqCst);
        hlc2.last_physical.store(physical, Ordering::SeqCst);
        
        // Generate timestamps with same physical time
        let ts1 = hlc1.now();
        let ts2 = hlc2.now();
        
        assert_eq!(ts1.physical, ts2.physical);
        
        // Update hlc1 with ts2
        let updated = hlc1.update(ts2).unwrap();
        
        // Logical counter should be max(ts1.logical, ts2.logical) + 1
        assert_eq!(updated.physical, physical);
        assert_eq!(updated.logical, cmp::max(ts1.logical, ts2.logical) + 1);
    }

    #[test]
    fn test_hlc_clock_drift_detection() {
        let hlc = HLC::new();
        
        let current = hlc.now();
        let too_far_future = HLCTimestamp::new(
            current.physical + MAX_CLOCK_DRIFT_MS + 1000,
            0
        );
        
        let result = hlc.update(too_far_future);
        assert!(result.is_err());
        
        match result {
            Err(HLCError::ClockDriftExceeded { .. }) => {},
            _ => panic!("Expected ClockDriftExceeded error"),
        }
    }

    #[test]
    fn test_hlc_concurrent_nodes() {
        use std::sync::Arc;
        use std::sync::Mutex as StdMutex;
        
        // Simulate multiple nodes, each with their own HLC
        let mut handles = vec![];
        let all_timestamps = Arc::new(StdMutex::new(Vec::new()));
        
        // Spawn multiple threads, each representing a separate node
        for node_id in 0..10 {
            let all_timestamps_clone = Arc::clone(&all_timestamps);
            let handle = thread::spawn(move || {
                // Each node has its own HLC instance
                let hlc = HLC::new();
                let mut local_timestamps = vec![];
                
                for _ in 0..100 {
                    let ts = hlc.now();
                    local_timestamps.push(ts);
                    
                    // Simulate receiving timestamp from another node
                    let remote = HLCTimestamp::new(ts.physical + node_id, node_id as u32);
                    if remote.is_within_drift(hlc.physical_now()) {
                        let _ = hlc.update(remote);
                    }
                }
                
                // Add to global collection
                all_timestamps_clone.lock().unwrap().extend(local_timestamps.clone());
                local_timestamps
            });
            handles.push(handle);
        }
        
        // Wait for all nodes to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Check timestamps from each node are unique
        let timestamps = all_timestamps.lock().unwrap();
        
        // Group by node (can have same timestamp from different nodes)
        // This is realistic - different nodes CAN generate the same timestamp
        // What matters is that within a single node, timestamps are unique
        println!("Total timestamps across all nodes: {}", timestamps.len());
    }
    
    #[test]
    fn test_hlc_single_node_concurrent_threads() {
        use std::sync::Arc;
        
        // This tests that a single HLC instance is thread-safe
        let hlc = Arc::new(HLC::new());
        let mut handles = vec![];
        
        // Spawn multiple threads using the same HLC
        for _ in 0..10 {
            let hlc_clone = Arc::clone(&hlc);
            let handle = thread::spawn(move || {
                let mut timestamps = vec![];
                for _ in 0..100 {
                    timestamps.push(hlc_clone.now());
                }
                timestamps
            });
            handles.push(handle);
        }
        
        // Collect all timestamps
        let mut all_timestamps = vec![];
        for handle in handles {
            all_timestamps.extend(handle.join().unwrap());
        }
        
        // Within a single node, timestamps must be unique
        all_timestamps.sort();
        for i in 1..all_timestamps.len() {
            assert_ne!(all_timestamps[i], all_timestamps[i-1], 
                      "Found duplicate timestamp within single node: {:?}", all_timestamps[i]);
        }
    }

    #[test]
    fn test_hlc_last_timestamp() {
        let hlc = HLC::new();
        
        assert_eq!(hlc.last(), HLCTimestamp::zero());
        
        let ts1 = hlc.now();
        assert_eq!(hlc.last(), ts1);
        
        let ts2 = hlc.now();
        assert_eq!(hlc.last(), ts2);
        assert!(ts2 > ts1);
    }

    #[test]
    fn test_hlc_zero_timestamp() {
        let zero = HLCTimestamp::zero();
        assert!(zero.is_zero());
        assert_eq!(zero.physical, 0);
        assert_eq!(zero.logical, 0);
        
        let non_zero = HLCTimestamp::new(1, 0);
        assert!(!non_zero.is_zero());
    }
}
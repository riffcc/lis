use chrono::{DateTime, Duration, Utc};
use std::sync::atomic::{AtomicI64, Ordering};

pub type Timestamp = DateTime<Utc>;

#[derive(Debug)]
pub struct HybridClock {
    physical_offset: AtomicI64,
    logical_counter: AtomicI64,
}

impl HybridClock {
    pub fn new() -> Self {
        Self {
            physical_offset: AtomicI64::new(0),
            logical_counter: AtomicI64::new(0),
        }
    }
    
    pub fn now(&self) -> HybridTimestamp {
        let physical = Utc::now() + Duration::milliseconds(self.physical_offset.load(Ordering::SeqCst));
        let logical = self.logical_counter.fetch_add(1, Ordering::SeqCst);
        
        HybridTimestamp {
            physical,
            logical,
        }
    }
    
    pub fn update(&self, remote: &HybridTimestamp) {
        let local_physical = Utc::now();
        let diff = remote.physical.signed_duration_since(local_physical).num_milliseconds();
        
        if diff > 0 {
            self.physical_offset.fetch_max(diff, Ordering::SeqCst);
        }
        
        self.logical_counter.fetch_max(remote.logical + 1, Ordering::SeqCst);
    }
    
    pub fn sync_ntp(&self) -> crate::Result<()> {
        // TODO: Implement NTP sync
        Ok(())
    }
}

impl Default for HybridClock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct HybridTimestamp {
    pub physical: Timestamp,
    pub logical: i64,
}

impl HybridTimestamp {
    pub fn elapsed(&self) -> Duration {
        Utc::now().signed_duration_since(self.physical)
    }
}
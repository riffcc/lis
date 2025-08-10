// CRDT (Conflict-free Replicated Data Type) implementations for RHC

pub mod or_set;
pub mod pn_counter;
pub mod lww_register;
pub mod mv_register;
pub mod rga;
pub mod lease_state;

pub use or_set::ORSet;
pub use pn_counter::PNCounter;
pub use lww_register::LWWRegister;
pub use mv_register::MVRegister;
pub use rga::RGA;
pub use lease_state::LeaseStateCRDT;

use crate::rhc::hlc::HLCTimestamp;

/// Trait for all CRDTs in RHC
pub trait CRDT: Clone {
    /// Merge another CRDT into this one
    fn merge(&mut self, other: &Self);
    
    /// Check if this CRDT is causally before another
    fn happens_before(&self, other: &Self) -> bool;
}

/// A value with an associated HLC timestamp
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampedValue<T> {
    pub value: T,
    pub timestamp: HLCTimestamp,
}

impl<T> TimestampedValue<T> {
    pub fn new(value: T, timestamp: HLCTimestamp) -> Self {
        Self { value, timestamp }
    }
}

/// Actor ID for CRDT operations (node or consensus group ID)
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorId(pub String);

impl ActorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}
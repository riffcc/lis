// Leader lease management for RHC
//
// Provides exclusive write authority over filesystem paths with:
// - 30-second Raft-style leader leases
// - Hierarchical lease delegation
// - Pre-committed approval chains
// - Byzantine fault tolerance

pub mod lease;
pub mod manager;
pub mod state;

pub use lease::{Lease, LeaseId, LeaseScope};
pub use manager::LeaseManager;
pub use state::{LeaseState, LeaseStatus};
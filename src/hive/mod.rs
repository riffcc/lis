// Hive: The metadata service for Lis
//
// Manages file metadata, chunk locations, and replication policies
// Similar to MooseFS master but distributed via Athens consensus

pub mod metadata;
pub mod policy;

pub use metadata::{FileMetadata, ChunkMetadata, HiveService};
pub use policy::{ReplicationPolicy, ConsistencyLevel, WriteAckPolicy};
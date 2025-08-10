// Athens: The consensus layer for Lis
// 
// Provides RHC (Riff.CC Hierarchical Consensus) implementation
// that breaks CAP by combining Raft, BFT, and CRDTs

pub mod node;
pub mod network;

pub use node::{AthensNode, NodeConfig};
pub use network::{NetworkMessage, LatencySimulator};
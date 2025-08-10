use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Lease expired for domain {domain:?}")]
    LeaseExpired { domain: String },
    
    #[error("Lease conflict: domain {domain:?} already leased to {holder:?}")]
    LeaseConflict { domain: String, holder: crate::NodeId },
    
    #[error("Invalid lease proof")]
    InvalidLeaseProof,
    
    #[error("Byzantine fault detected from node {node:?}")]
    ByzantineFault { node: crate::NodeId },
    
    #[error("Insufficient shares for threshold signature: got {got}, need {need}")]
    InsufficientShares { got: usize, need: usize },
    
    #[error("Network partition detected")]
    NetworkPartition,
    
    #[error("Clock skew too high: {skew_ms}ms")]
    ClockSkew { skew_ms: i64 },
    
    #[error("Cryptographic error: {0}")]
    Crypto(String),
    
    #[error("Storage error: {0}")]
    Storage(String),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
#![forbid(unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod consensus;
pub mod crypto;
pub mod error;
pub mod lease;
pub mod message;
pub mod node;
pub mod storage;
pub mod time;

pub mod test_utils;

pub use error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsistencyLevel {
    Local,
    Regional,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(pub uuid::Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

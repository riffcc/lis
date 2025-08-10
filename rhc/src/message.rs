use crate::{
    crypto::Signature,
    lease::{Domain, LeaseProof},
    time::HybridTimestamp,
    NodeId,
};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // Lease management
    LeaseRequest(LeaseRequest),
    LeaseGrant(LeaseGrant),
    LeaseRenew(LeaseRenew),
    LeaseRevoke(LeaseRevoke),
    
    // Consensus messages
    Propose(ConsensusProposal),
    ThresholdShare(ThresholdShare),
    Commit(CommitProof),
    
    // Synchronization
    Heartbeat(Heartbeat),
    SyncBatch(SyncBatch),
    
    // Byzantine fault tolerance
    ViewChange(ViewChange),
    Accusation(Accusation),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRequest {
    pub request_id: Uuid,
    pub domain: Domain,
    pub duration: Duration,
    pub requester: NodeId,
    pub parent_lease_proof: Option<LeaseProof>,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseGrant {
    pub request_id: Uuid,
    pub lease_proof: LeaseProof,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRenew {
    pub lease_id: Uuid,
    pub duration: Duration,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRevoke {
    pub lease_id: Uuid,
    pub reason: String,
    pub signature: Signature,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusProposal {
    pub round: u64,
    pub value: Vec<u8>,
    pub proposer: NodeId,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdShare {
    pub round: u64,
    pub node_id: NodeId,
    pub share: Signature,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitProof {
    pub round: u64,
    pub value: Vec<u8>,
    pub aggregated_signature: Signature,
    pub signers: Vec<NodeId>,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub node_id: NodeId,
    pub epoch: u64,
    pub active_leases: Vec<Uuid>,
    pub load: LoadInfo,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadInfo {
    pub cpu_usage: f32,
    pub memory_usage: f32,
    pub pending_operations: u64,
    pub latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatch {
    pub source: NodeId,
    pub domain: Domain,
    pub operations: Vec<Operation>,
    pub checkpoint: Checkpoint,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: Uuid,
    pub op_type: OperationType,
    pub data: Vec<u8>,
    pub lease_proof: LeaseProof,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationType {
    Write,
    Delete,
    Rename,
    CreateDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub epoch: u64,
    pub state_hash: [u8; 32],
    pub operation_count: u64,
    pub signature: Signature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewChange {
    pub old_view: u64,
    pub new_view: u64,
    pub node_id: NodeId,
    pub reason: String,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Accusation {
    pub accuser: NodeId,
    pub accused: NodeId,
    pub evidence: Evidence,
    pub timestamp: HybridTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Evidence {
    ConflictingMessages(Box<Message>, Box<Message>),
    InvalidSignature(Box<Message>),
    ProtocolViolation(String),
}
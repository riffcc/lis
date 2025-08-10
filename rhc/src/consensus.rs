use crate::{
    crypto::{BlsKeyPair, ThresholdSignatureAggregator},
    message::{CommitProof, ConsensusProposal, Message, ThresholdShare},
    time::HybridClock,
    NodeId, Result,
};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct BftConsensus {
    node_id: NodeId,
    round: Arc<RwLock<u64>>,
    threshold: usize,
    total_nodes: usize,
    proposals: Arc<DashMap<u64, ConsensusProposal>>,
    shares: Arc<DashMap<u64, ThresholdSignatureAggregator>>,
    commits: Arc<DashMap<u64, CommitProof>>,
    clock: Arc<HybridClock>,
    keypair: BlsKeyPair,
    message_tx: mpsc::UnboundedSender<Message>,
}

impl BftConsensus {
    pub fn new(
        node_id: NodeId,
        threshold: usize,
        total_nodes: usize,
        message_tx: mpsc::UnboundedSender<Message>,
    ) -> Self {
        assert!(threshold >= (total_nodes * 2 / 3) + 1, "Threshold must be at least 2f+1");
        
        Self {
            node_id,
            round: Arc::new(RwLock::new(0)),
            threshold,
            total_nodes,
            proposals: Arc::new(DashMap::new()),
            shares: Arc::new(DashMap::new()),
            commits: Arc::new(DashMap::new()),
            clock: Arc::new(HybridClock::new()),
            keypair: BlsKeyPair::generate(),
            message_tx,
        }
    }
    
    pub async fn propose(&self, value: Vec<u8>) -> Result<()> {
        let round = {
            let mut r = self.round.write();
            *r += 1;
            *r
        };
        
        let proposal = ConsensusProposal {
            round,
            value,
            proposer: self.node_id,
            timestamp: self.clock.now(),
        };
        
        self.proposals.insert(round, proposal.clone());
        
        // Flood proposal
        self.message_tx.send(Message::Propose(proposal))
            .map_err(|_| crate::Error::Other(anyhow::anyhow!("Failed to send message")))?;
        
        // Immediately sign our own proposal
        self.sign_proposal(round).await?;
        
        Ok(())
    }
    
    pub async fn handle_proposal(&self, proposal: ConsensusProposal) -> Result<()> {
        // Verify proposal is for a new round
        let current_round = *self.round.read();
        if proposal.round <= current_round {
            return Ok(()); // Ignore old proposals
        }
        
        // Store proposal
        self.proposals.insert(proposal.round, proposal.clone());
        
        // Sign the proposal
        self.sign_proposal(proposal.round).await?;
        
        Ok(())
    }
    
    async fn sign_proposal(&self, round: u64) -> Result<()> {
        let proposal = self.proposals.get(&round)
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("Proposal not found")))?;
        
        // Create message to sign: hash(round || value)
        let mut message = Vec::new();
        message.extend_from_slice(&round.to_le_bytes());
        message.extend_from_slice(&proposal.value);
        let hash = crate::crypto::hash(&message);
        
        // Generate threshold signature share
        let share = self.keypair.sign(&hash);
        
        let threshold_share = ThresholdShare {
            round,
            node_id: self.node_id,
            share,
            timestamp: self.clock.now(),
        };
        
        // Store our own share
        self.shares.entry(round)
            .or_insert_with(|| ThresholdSignatureAggregator::new(self.threshold))
            .add_share(self.node_id, &threshold_share.share)?;
        
        // Flood share continuously
        self.message_tx.send(Message::ThresholdShare(threshold_share))
            .map_err(|_| crate::Error::Other(anyhow::anyhow!("Failed to send message")))?;
        
        // Check if we have enough shares
        self.try_aggregate(round).await?;
        
        Ok(())
    }
    
    pub async fn handle_share(&self, share: ThresholdShare) -> Result<()> {
        // Verify we have the proposal
        if !self.proposals.contains_key(&share.round) {
            return Ok(()); // Ignore shares for unknown proposals
        }
        
        // Add share to aggregator
        self.shares.entry(share.round)
            .or_insert_with(|| ThresholdSignatureAggregator::new(self.threshold))
            .add_share(share.node_id, &share.share)?;
        
        // Try to aggregate if we have threshold
        self.try_aggregate(share.round).await?;
        
        Ok(())
    }
    
    async fn try_aggregate(&self, round: u64) -> Result<()> {
        let aggregator = self.shares.get(&round)
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("No shares for round")))?;
        
        if !aggregator.has_threshold() {
            return Ok(());
        }
        
        // Already committed?
        if self.commits.contains_key(&round) {
            return Ok(());
        }
        
        let proposal = self.proposals.get(&round)
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("Proposal not found")))?;
        
        // Aggregate signatures
        let aggregated_signature = aggregator.aggregate()?;
        
        let commit = CommitProof {
            round,
            value: proposal.value.clone(),
            aggregated_signature,
            signers: vec![], // TODO: Collect actual signers
            timestamp: self.clock.now(),
        };
        
        // Store commit
        self.commits.insert(round, commit.clone());
        
        // Flood commit once
        self.message_tx.send(Message::Commit(commit))
            .map_err(|_| crate::Error::Other(anyhow::anyhow!("Failed to send message")))?;
        
        // Update round
        let mut current_round = self.round.write();
        if round > *current_round {
            *current_round = round;
        }
        
        Ok(())
    }
    
    pub async fn handle_commit(&self, commit: CommitProof) -> Result<()> {
        // Verify commit signature
        // TODO: Implement signature verification
        
        // Store commit
        self.commits.insert(commit.round, commit.clone());
        
        // Update round
        let mut current_round = self.round.write();
        if commit.round > *current_round {
            *current_round = commit.round;
        }
        
        Ok(())
    }
    
    pub fn get_committed_value(&self, round: u64) -> Option<Vec<u8>> {
        self.commits.get(&round).map(|c| c.value.clone())
    }
    
    pub fn current_round(&self) -> u64 {
        *self.round.read()
    }
}
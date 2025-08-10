# Consensus Groups in RHC

## Overview

Consensus Groups (CGs) are the fundamental distributed state machines in RHC. Each CG is a replicated state machine that maintains consistent state across multiple nodes using a hybrid of Raft (for normal operations) and BFT (for critical operations).

## Key Properties

1. **State Machine Replication**: Each CG maintains identical state across all non-faulty replicas
2. **Hybrid Consensus**: Uses Raft for efficiency, BFT for security
3. **Dynamic Membership**: Nodes can join/leave groups dynamically
4. **Partition Tolerance**: Uses CRDTs to merge state after network partitions
5. **Lease Management**: CGs ARE the distributed store for lease state

## Architecture

### Group Structure
```
ConsensusGroup {
    id: GroupId,
    members: Vec<NodeId>,
    leader: Option<NodeId>,
    epoch: u64,
    state: Box<dyn StateMachine>,
    consensus_mode: ConsensusMode,
}
```

### Consensus Modes
- **Raft Mode**: Default for normal operations (fast, requires non-Byzantine majority)
- **BFT Mode**: For critical operations like lease migration (slower, tolerates Byzantine failures)
- **Degraded Mode**: When insufficient replicas, read-only with CRDTs

## Membership Management

### Join Protocol
1. New node contacts any existing member
2. Existing member forwards join request to leader
3. Leader proposes membership change through consensus
4. New node receives state snapshot
5. New node begins participating in consensus

### Leave Protocol
1. Node announces intent to leave
2. Leader initiates state transfer to maintain replication factor
3. Membership change committed through consensus
4. Node stops participating

### Failure Detection
- Heartbeat-based failure detection
- Configurable timeout (default: 150ms for LAN, 1s for WAN)
- Suspected failures trigger view change

## Leader Election

### Raft Leader Election (Normal Mode)
1. Followers timeout on missing heartbeats
2. Candidate increments term and requests votes
3. Majority vote wins election
4. New leader sends heartbeats to establish authority

### BFT View Change (Byzantine Mode)
1. Replicas timeout on missing progress
2. View change protocol initiated
3. 2f+1 replicas must agree on new view
4. New primary selected deterministically

## State Machine Interface

```rust
trait StateMachine: Send + Sync {
    /// Apply a command to the state machine
    fn apply(&mut self, command: Command) -> Result<Response>;
    
    /// Take a snapshot for replication
    fn snapshot(&self) -> Snapshot;
    
    /// Restore from snapshot
    fn restore(&mut self, snapshot: Snapshot) -> Result<()>;
    
    /// Merge with another state (for CRDT operations)
    fn merge(&mut self, other: &Self) -> Result<()>;
}
```

## Replication Protocol

### Normal Path (Raft)
1. Client sends request to leader
2. Leader appends to log and replicates to followers
3. Followers acknowledge
4. Leader commits after majority ack
5. Leader applies to state machine and responds to client

### Byzantine Path (BFT)
1. Client sends request to all replicas
2. Primary orders request and sends pre-prepare
3. Replicas validate and send prepare messages
4. After 2f+1 prepares, replicas send commit
5. After 2f+1 commits, execute and reply to client

## Partition Handling

### During Partition
- Each partition continues operating if it has quorum
- Conflicting operations tracked with HLC timestamps
- CRDTs accumulate divergent state

### After Partition Heal
1. Nodes discover each other via gossip
2. Exchange vector clocks to identify divergence
3. Exchange CRDT state for divergent period
4. Merge states using CRDT semantics
5. Resume normal consensus operation

## Integration Points

### With HLC
- All operations timestamped with HLC
- Provides global ordering even during partitions
- Enables deterministic conflict resolution

### With Leases
- CGs maintain lease state as primary function
- Lease operations go through consensus
- Lease validity checked against HLC timestamps

### With CRDTs
- State machines implement CRDT merge semantics
- Enables automatic conflict resolution
- Preserves all operations during partitions

## Example: Lease Management CG

```rust
struct LeaseStateMachine {
    leases: HashMap<LeaseScope, LeaseInfo>,
    hlc: Arc<HLC>,
}

impl StateMachine for LeaseStateMachine {
    fn apply(&mut self, command: Command) -> Result<Response> {
        match command {
            Command::RequestLease { scope, duration, holder } => {
                // Check if lease available
                if let Some(existing) = self.leases.get(&scope) {
                    if self.hlc.now() < existing.expires_at {
                        return Err(LeaseUnavailable);
                    }
                }
                
                // Grant lease
                let lease = LeaseInfo {
                    holder,
                    granted_at: self.hlc.now(),
                    expires_at: self.hlc.now() + duration,
                };
                
                self.leases.insert(scope, lease.clone());
                Ok(Response::LeaseGranted(lease))
            }
            // ... other commands
        }
    }
}
```

## Performance Characteristics

### Latency
- Raft mode: 1 RTT for reads (from leader), 1-2 RTT for writes
- BFT mode: 1 RTT for reads (from f+1 replicas), 3-4 RTT for writes
- Partition recovery: Depends on divergence amount

### Throughput
- Raft mode: ~100k ops/sec (limited by leader)
- BFT mode: ~10k ops/sec (limited by message complexity)
- Degraded mode: Read-only, unlimited read throughput

### Scalability
- Optimal group size: 5-7 nodes (balances fault tolerance and performance)
- Can support 100s of consensus groups per physical node
- Groups can be geo-distributed with appropriate timeouts

## Configuration

### Recommended Settings
```toml
[consensus_group]
# Number of replicas
replication_factor = 5

# Failure detection timeout
heartbeat_timeout_ms = 150  # LAN
# heartbeat_timeout_ms = 1000  # WAN

# When to switch to BFT mode
bft_threshold = "lease_migration"

# Maximum divergence before forcing reconciliation
max_divergence_duration_ms = 30000
```

## Implementation Status

- [ ] Basic CG structure and interfaces
- [ ] Raft consensus integration
- [ ] BFT consensus integration  
- [ ] Dynamic membership
- [ ] Partition detection and recovery
- [ ] CRDT merge protocols
- [ ] Monitoring and observability
# Consensus Group Membership Management

## Overview

Membership management is critical for consensus groups to handle nodes joining, leaving, and failing. RHC uses a hybrid approach combining Raft's joint consensus for planned changes and BFT protocols for Byzantine fault tolerance.

## Membership States

### Node States
```rust
enum NodeState {
    Joining,      // New node syncing state
    Active,       // Full participant in consensus  
    Leaving,      // Graceful departure in progress
    Failed,       // Detected as failed by group
    Quarantined,  // Suspected Byzantine, isolated
}
```

### Group Membership View
```rust
struct MembershipView {
    epoch: u64,                    // Monotonic version number
    members: HashMap<NodeId, NodeState>,
    leader: Option<NodeId>,        
    configuration_id: Uuid,        // Unique config identifier
    timestamp: HLCTimestamp,       // When view was created
}
```

## Join Protocol

### 1. Discovery Phase
```
NewNode -> AnyMember: JoinRequest { 
    node_id, 
    capabilities,
    proof_of_resources 
}
AnyMember -> Leader: ForwardedJoinRequest
```

### 2. Validation Phase
- Leader verifies node identity (cryptographic proof)
- Checks resource requirements (storage, bandwidth)
- Validates no conflicts with existing members
- Ensures group size limits not exceeded

### 3. State Transfer Phase
```
Leader -> NewNode: StateSnapshot {
    snapshot_index,
    state_data,
    recent_log_entries
}
NewNode -> Leader: SnapshotAck
```

### 4. Transition Phase
- Uses joint consensus (Cjoint = Cold ∪ Cnew)
- Requires agreement from both old and new configurations
- Prevents split-brain during reconfiguration

### 5. Activation Phase
```
Leader -> AllNodes: ConfigurationCommit {
    new_membership_view
}
AllNodes -> Leader: CommitAck
```

## Leave Protocol

### Planned Leave
1. **Announcement**: Node declares intent to leave
2. **Load Migration**: Transfer responsibilities to remaining nodes
3. **State Verification**: Ensure no data loss
4. **Configuration Change**: Remove via joint consensus
5. **Cleanup**: Release resources, update routing

### Emergency Leave
- Triggered by impending hardware failure
- Abbreviated protocol focusing on data safety
- May use BFT mode for faster consensus

## Failure Detection

### Heartbeat Mechanism
```rust
struct Heartbeat {
    from: NodeId,
    term: u64,
    timestamp: HLCTimestamp,
    load_info: LoadMetrics,
}
```

### Detection Thresholds
- LAN: 3 missed heartbeats (450ms)
- WAN: 5 missed heartbeats (5s)
- Adjustable based on network conditions

### Suspicion Protocol
1. Node A misses heartbeats from Node B
2. A marks B as suspected
3. A queries other nodes about B
4. If majority agree, B marked as failed
5. Membership reconfiguration initiated

## Byzantine Fault Handling

### Detection Mechanisms
- Cryptographic proof validation failures
- Inconsistent state reports
- Protocol violation detection
- Behavioral anomaly detection

### Quarantine Process
1. Suspicious behavior detected
2. Node placed in quarantine (can read, cannot write)
3. Audit performed by trusted nodes
4. Either cleared or permanently expelled

### BFT Mode Trigger
- Automatic when Byzantine behavior detected
- Manual by administrator
- During critical operations (data migration)

## Split-Brain Prevention

### Majority Requirement
- Configuration changes require majority from BOTH old and new configs
- Prevents two groups from diverging

### Epoch Numbers
- Each configuration has monotonically increasing epoch
- Nodes reject messages from older epochs
- Ensures single active configuration

### Network Partition Handling
```rust
fn can_make_progress(&self) -> bool {
    let active_nodes = self.count_active_nodes();
    let total_nodes = self.membership.len();
    
    // Require strict majority
    active_nodes > total_nodes / 2
}
```

## Dynamic Scaling

### Scale-Up Triggers
- Load exceeds threshold (CPU, memory, storage)
- Latency degradation detected
- Administrator initiated

### Scale-Down Triggers
- Underutilization for sustained period
- Cost optimization
- Maintenance requirements

### Rebalancing
- Triggered after membership changes
- Redistributes data evenly
- Minimizes data movement
- Maintains availability during rebalance

## Performance Impact

### During Reconfiguration
- Read performance: Unaffected (served by any node)
- Write performance: ~20% degradation (joint consensus overhead)
- Duration: Typically 5-30 seconds depending on state size

### Optimization Strategies
1. **Batch Changes**: Group multiple membership changes
2. **Off-Peak Scheduling**: Perform during low-traffic periods
3. **Pre-Transfer**: Start state transfer before configuration change
4. **Incremental Sync**: Transfer only delta after initial snapshot

## Configuration Limits

### Recommended Limits
```toml
[membership]
min_nodes = 3              # Minimum for fault tolerance
max_nodes = 7              # Maximum for performance
max_join_rate = 1/min      # Prevent thrashing
max_leave_rate = 1/min     # Ensure stability
quarantine_timeout = 1h    # Auto-expel after timeout
```

### Anti-Patterns to Avoid
1. Rapid membership changes (causes instability)
2. Even number of nodes (no tie-breaker)  
3. Geographically unbalanced distribution
4. Mixing vastly different hardware

## Monitoring and Alerts

### Key Metrics
- Membership change frequency
- Reconfiguration duration
- Failed node detection time
- State transfer bandwidth

### Alert Conditions
- Frequent membership changes
- Failed reconfigurations
- Stuck in joint consensus
- Byzantine behavior detected

## Implementation Checklist

- [ ] Basic join/leave protocols
- [ ] Joint consensus implementation
- [ ] Failure detection with timeouts
- [ ] Byzantine fault detection
- [ ] State transfer optimization
- [ ] Network partition handling
- [ ] Dynamic scaling policies
- [ ] Monitoring and metrics
- [ ] Administrative tools
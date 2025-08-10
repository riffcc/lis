# RHC Leader Leases

## Overview

Leader leases in RHC provide exclusive write authority over specific filesystem paths. Based on Raft-style leader election with Byzantine Fault Tolerance, leases ensure strong consistency for writes while allowing unrestricted local reads.

## Core Concepts

### Lease Properties

- **Duration**: 30-second leases (configurable)
- **Scope**: Per file or folder (MooseFS-style granularity)
- **Authority**: Exclusive WRITE permission (reads always local)
- **Validation**: Raft-style - requires ongoing validation from peers

### Lease Lifecycle

1. **Acquisition**: CG requests lease through consensus
2. **Validation**: Peers validate and acknowledge leadership
3. **Renewal**: Can occur anytime during 30s window, especially on major writes
4. **Expiration**: Lease holder must halt writes if unable to renew

## Hierarchical Lease Management

### Nesting and Specificity

Leases can be nested with "more specific wins" semantics:

```
/mnt/lis/lon              - Held by London CG
/mnt/lis/lon/media        - Held by Bedroom CG  
/mnt/lis/lon/documents    - Held by Office CG
```

Similar to MooseFS goals, you can:
- Set leases at any directory level
- Override parent leases with more specific child leases
- Apply leases recursively down the tree

### Consensus Group Participation

CGs participate in lease elections through recursive consensus:

1. Individual nodes within a CG build consensus for CG decisions
2. CG requests lease on behalf of its members
3. Recursive lease delegation down to the specific CG needing the lease
4. Low-latency handoff using pre-committed approvals

### Pre-Committed Approvals and Low-Latency Delegation

The key innovation in RHC's lease system is **pre-encoded conditional decisions**. Rather than requiring multiple rounds of consensus at each hierarchy level, nodes pre-commit to conditional approvals:

```
Global CG:    "IF Europe requests /data/eu, THEN approved"
Europe CG:    "IF UK requests /data/eu/uk, THEN approved"  
UK CG:        "IF London requests /data/eu/uk/lon, THEN approved"
```

When London needs a lease on `/data/eu/uk/lon/documents`:

1. **Request Creation**: London creates lease request
2. **Approval Collection**: As the request travels up, each CG adds its pre-committed approval
3. **Single Traversal**: Request carries all approvals in one network path
4. **Fast Validation**: Target receives request WITH complete approval chain

This transforms lease acquisition from O(n) consensus rounds to O(1) network traversal:
- No back-and-forth negotiations
- No waiting for each level to vote
- Cryptographic signatures validate entire delegation path
- Minimal additional latency beyond required network hops

### Information Propagation

Nodes maintain lease awareness through limited scope:
- Direct parents
- Direct children  
- Siblings

This bounded communication keeps the system scalable while maintaining consistency.

## Failure Handling

### Lease Validation

Following Raft principles:
- Lease holders need continuous validation from peers
- A node claiming an expired lease is ignored by the network
- No voting or consensus building occurs on invalid leases

### Network Partitions

BFT consensus (two-flood protocol) ensures:
- Clean leader election with no overlaps
- Partition-tolerant operation
- No split-brain scenarios (two nodes can never share a lease)

## Implementation Considerations

### Renewal Strategy

Leases can be renewed:
- On any major write operation
- Proactively before expiration
- At any point during the 30-second window

### Write Halting

If a lease holder cannot renew:
1. Must immediately halt all write operations
2. Continue serving reads (always allowed)
3. Re-enter consensus to reacquire lease

### Recursive Delegation

The lease system supports efficient delegation through:
- Pre-committed approvals that aggregate consensus decisions
- Low latency for nested lease requests
- Cryptographic signatures validating the delegation path

The pre-encoded conditional approvals mean that:
- Each CG can specify its delegation policies in advance
- These policies travel with requests as cryptographic proofs
- No CG needs to wait for explicit approval from another CG
- The entire hierarchy can process a lease request in a single pass

## Integration with RHC Components

- **HLC**: Timestamps track lease start/expiration times
- **BFT Consensus**: Ensures unique lease holders via two-flood protocol
- **CRDTs**: Handle conflicts during lease transitions
- **Gossip**: Propagates lease state changes to relevant nodes
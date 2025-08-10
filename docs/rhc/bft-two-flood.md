# BFT Two-Flood Protocol

## Overview

RHC uses a streamlined Byzantine Fault Tolerant consensus protocol based on two message floods rather than traditional view-change mechanisms. This achieves consensus in optimal rounds while maintaining safety under Byzantine failures.

## Protocol Design

### Participants

- **Proposer**: Node initiating a value
- **Arbitrators**: Fixed set of 3f+1 nodes (tolerates f Byzantine)
- **Threshold**: 2f+1 signatures required for commit

### Message Types

```json
// Phase 1: Proposal flood
{
  "type": "PROPOSE",
  "round": 911,
  "value": {
    "path": "/data/users.db",
    "op": "write",
    "data": "users: alice, bob, charlie",
    "lease_proof": "bls:0x..."
  },
  "proposer": "london-cg",
  "signature": "bls:0x..."
}

// Phase 2: Share flood
{
  "type": "SHARE",
  "round": 911,
  "arbitrator": "arb-3",
  "value_hash": "blake3:0x...",
  "share": "bls_share:0x...",
  "signature": "bls:0x..."
}

// Final: Commit proof
{
  "type": "COMMIT",
  "round": 911,
  "value": { "..." },
  "proof": "bls_aggregate:0x...",
  "arbitrators": ["arb-1", "arb-3", "arb-5", "..."],
  "committee_hash": "blake3:0x..."
}
```

## Two-Flood Sequence

### Round 1: PROPOSE Flood

1. Proposer broadcasts PROPOSE to all arbitrators
2. Message includes:
   - Value to commit
   - Valid lease proof for write authority
   - Round number (monotonic)
3. Flood ensures message reaches all non-faulty nodes

### Round 2: SHARE Flood

1. Each arbitrator receiving valid PROPOSE:
   - Verifies lease proof
   - Checks round number is current
   - Creates BLS signature share
   - Floods SHARE to all nodes

2. Share validation:
   - Must reference correct value hash
   - Must be from authorized arbitrator
   - Must have valid signature

### Aggregation: COMMIT Proof

1. Any node collecting 2f+1 valid shares:
   - Aggregates BLS signatures into single proof
   - Creates COMMIT message with proof
   - Floods to all nodes

2. Properties:
   - Single compact signature (48 bytes)
   - Proves 2f+1 arbitrators agreed
   - Non-repudiable commitment

## Handling Network Asymmetry

The two-flood design handles asymmetric partitions elegantly:

```
Scenario: Perth can reach London but not NewYork

Round 1: Perth floods PROPOSE
- London receives ✓
- NewYork misses ✗

Round 2: London floods SHARE
- Perth receives ✓
- NewYork receives ✓ (from London)

Result: NewYork gets shares via London relay
```

As long as the arbitrator graph has sufficient connectivity, shares propagate through alternate paths.

## Optimizations

### 1. Threshold Signatures
- Use BLS12-381 for aggregatable signatures
- 2f+1 shares → single 48-byte proof
- Verification time: ~2ms

### 2. Round Coordination
- Rounds advance on timeout or commit
- No explicit view-change protocol
- Natural leader rotation via round robin

### 3. Batching
- Multiple operations per round
- Amortize signature costs
- Typical batch: 100-1000 ops

## Safety Properties

### Agreement
- No two different values can get 2f+1 shares in same round
- Guaranteed by threshold signatures

### Validity
- Only values with valid lease proofs can be proposed
- Enforced by arbitrator verification

### Termination
- Completes in 2 rounds under partial synchrony
- No infinite view-change loops

## Performance Characteristics

- **Latency**: 2 × network RTT
- **Message complexity**: O(n²) for n arbitrators
- **Signature overhead**: 1 sign + 1 verify per arbitrator
- **Proof size**: 48 bytes (constant)

## Integration with RHC

1. **Lease Operations**: Use BFT for cross-CG lease grants
2. **Critical Writes**: BFT commit for geo-distributed durability
3. **Reconfigurations**: BFT consensus on CG membership changes

## Comparison to Classical BFT

| Aspect | Classical PBFT | Two-Flood BFT |
|--------|---------------|---------------|
| Rounds | 3 (prepare, commit, reply) | 2 (propose, share) |
| View Changes | Complex protocol | Natural timeout |
| Message Size | O(n) signatures | O(1) aggregate |
| Partition Tolerance | Requires majority | Works with connectivity |
# Lease Fencing and Atomic Migration

## Overview

Lease fencing ensures clean handoffs during lease migration, node failures, or reconfigurations. The protocol prevents split-brain by atomically transitioning from one lease holder to another with no overlap period.

## Fencing Protocol

### 1. Initiate Fence

When a lease needs to be revoked (migration, failure, expiry):

```json
{
  "type": "FENCE_REQUEST",
  "domain": "/data/users/",
  "current_lease_id": "london-1754842710000",
  "reason": "migration_to_perth",
  "requester": "perth-cg",
  "timestamp": "1754842735000:0"
}
```

### 2. Parent CG Issues Fence

The parent CG (issuer of original lease) validates and signs:

```json
{
  "type": "FENCE_CERTIFICATE",
  "domain": "/data/users/",
  "fenced_lease_id": "london-1754842710000",
  "fence_time": "1754842735500:0",
  "issuer": "europe-cg",
  "next_holder": "perth-cg",
  "signature": "bls:0x8f3a2...",
  "propagation": "flood"
}
```

### 3. Propagation

Fence certificates are flooded via gossip to ensure all nodes learn:
- Previous lease holder must stop writes immediately
- No new operations accepted with fenced lease
- Readers can continue (reads always allowed)

### 4. New Lease Grant

Only after fence is confirmed, parent issues new lease:

```json
{
  "type": "LEASE_GRANT",
  "domain": "/data/users/",
  "holder": "perth-cg",
  "start_ms": 1754842736000,
  "expiry_ms": 1754842766000,
  "predecessor": {
    "lease_id": "london-1754842710000",
    "fence_proof": "bls:0x8f3a2..."
  },
  "signature": "bls:0x5c8d4..."
}
```

## Atomic Migration Sequence

### Step 1: Pre-Migration Sync
```
Perth → Europe-CG: REQUEST_LEASE(/data/users/)
Europe-CG: CHECK lease table, London holds until 1754842740000
Europe-CG → Perth: WAIT or REQUEST_MIGRATION
```

### Step 2: Coordinated Handoff
```
Perth → Europe-CG: REQUEST_MIGRATION(/data/users/)
Europe-CG → London: FENCE_CERTIFICATE (signed)
London: STOPS writes, FLUSHES pending ops
London → Europe-CG: FENCE_ACKNOWLEDGED + final state hash
```

### Step 3: State Transfer
```
Europe-CG → Perth: LEASE_GRANT + London's final state hash
Perth ← London: SYNC final operations (pull)
Perth: VERIFIES state hash matches
Perth → Europe-CG: LEASE_ACTIVE confirmation
```

## Handling Asymmetric Partitions

When London → Perth link is broken but Perth → London works:

### Use Relay Fencing

```
1. Perth → Europe-CG: REQUEST_MIGRATION
2. Europe-CG → NewYork: RELAY_FENCE(London, fence_cert)
3. NewYork → London: FENCE_CERTIFICATE
4. London → NewYork: FENCE_ACK + state
5. NewYork → Perth: RELAY_STATE(London's final state)
6. Europe-CG → Perth: LEASE_GRANT
```

### Fence Flooding

Even if direct paths fail, fence floods through alternate routes:
- Each node receiving fence rebroadcasts
- Exponential backoff prevents storms
- Fence is idempotent (can receive multiple times)

## Recovery After Fencing

### Scenario: London was fenced but partition healed

```rust
fn handle_post_fence_write(lease_proof: &LeaseProof) -> Result<(), Error> {
    // Check if lease is fenced
    if fence_table.is_fenced(&lease_proof.lease_id) {
        return Err(Error::LeaseFenced {
            lease_id: lease_proof.lease_id.clone(),
            fence_time: fence_table.get_fence_time(&lease_proof.lease_id),
        });
    }
    
    // Check if we have a successor lease
    if let Some(successor) = lease_table.get_successor(&lease_proof.domain) {
        return Err(Error::LeaseSuperseded {
            old: lease_proof.lease_id.clone(),
            new: successor.lease_id.clone(),
        });
    }
    
    // Safe to proceed
    Ok(())
}
```

## Burst Buffer Handling

During fence transition:

1. **Old Holder**: Flushes burst buffer before acknowledging fence
2. **Fence Period**: Burst buffer accepts reads only
3. **New Holder**: Imports old holder's final buffer state

```rust
impl BurstBuffer {
    async fn handle_fence(&mut self, fence: FenceCertificate) -> FinalState {
        // Stop accepting writes
        self.read_only = true;
        
        // Flush pending operations
        let pending = self.drain_pending().await;
        
        // Create final state snapshot
        let final_state = FinalState {
            operations: pending,
            state_hash: self.compute_hash(),
            fence_ack_time: HLC::now(),
        };
        
        // Mark as fenced
        self.fenced = true;
        
        final_state
    }
}
```

## Fence Validation

All nodes must validate fence certificates:

```rust
fn validate_fence(fence: &FenceCertificate) -> Result<(), Error> {
    // 1. Verify signature from authorized issuer
    if !verify_bls_signature(&fence.issuer, &fence.signature, fence.hash()) {
        return Err(Error::InvalidFenceSignature);
    }
    
    // 2. Check issuer has authority over domain
    if !authority_table.can_fence(&fence.issuer, &fence.domain) {
        return Err(Error::UnauthorizedFence);
    }
    
    // 3. Verify timeline (fence after lease start)
    if fence.fence_time <= lease_table.get_start_time(&fence.fenced_lease_id) {
        return Err(Error::FenceBeforeLease);
    }
    
    // 4. Check not already superseded
    if fence_table.has_newer_fence(&fence.domain, &fence.fence_time) {
        return Err(Error::FenceSuperseded);
    }
    
    Ok(())
}
```

## Integration with Two-Generals

For critical fences across unreliable links, use commitment flooding:

1. **Initiator** continuously floods: "I want to fence X"
2. **Parent** continuously floods: "I will fence X"
3. **Initiator** continuously floods: "I see you will fence X"

Once triple-proof is constructible by either party, fence proceeds even if final message is lost.

## Fence Durability

Fences must be persisted durably:
- Write to local persistent storage
- Replicate to quorum before acknowledging
- Include in periodic checkpoints
- Never garbage collect active fences

## Performance Impact

- **Fence Latency**: 1 RTT to parent + flood propagation
- **Migration Downtime**: ~100ms typical (flush + fence + grant)
- **Fence Certificate Size**: ~200 bytes
- **Flood Convergence**: O(log n) rounds typical
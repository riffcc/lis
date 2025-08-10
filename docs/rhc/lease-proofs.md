# Cryptographic Lease Proofs

## Overview

Every lease in RHC carries a cryptographic proof establishing its validity. This prevents split-brain scenarios and ensures only authorized nodes can claim write authority.

## Lease Proof Structure

```json
{
  "domain": "/data/users/",
  "holder": "london-cg",
  "start_ms": 1754842710000,
  "expiry_ms": 1754842740000,
  "parent": "europe-cg",
  "parent_signature": "bls:0x3f4a5b...",
  "lease_id": "london-1754842710000",
  "predecessor": "paris-1754842680000",
  "predecessor_fence": "bls:0x8a9c2..."
}
```

### Fields

- **domain**: Filesystem path this lease covers
- **holder**: CG/node authorized to write
- **start_ms/expiry_ms**: Lease validity window (HLC timestamps)
- **parent**: CG that issued this lease
- **parent_signature**: BLS signature from parent CG
- **lease_id**: Unique identifier (holder + start time)
- **predecessor**: Previous lease being replaced (if any)
- **predecessor_fence**: Proof that previous lease was properly fenced

## Lease Issuance Protocol

1. **Request Phase**
   - Requester sends `LEASE_REQUEST` to parent CG
   - Includes domain, requested duration, proof of need

2. **Validation Phase**
   - Parent checks lease table for conflicts
   - Verifies no overlapping active leases
   - Atomic CAS operation on lease table

3. **Issuance Phase**
   - Parent creates lease proof with signature
   - Updates lease table atomically
   - Returns signed proof to requester

## Preventing Split-Brain

### Atomic Lease Table

Each CG maintains a lease table with atomic compare-and-swap:

```rust
struct LeaseTable {
    // domain -> current lease
    leases: Arc<RwLock<HashMap<PathBuf, LeaseRecord>>>,
}

impl LeaseTable {
    fn grant_lease(&self, request: LeaseRequest) -> Result<LeaseProof, LeaseError> {
        let mut table = self.leases.write().unwrap();
        
        // Check for conflicts atomically
        if let Some(existing) = table.get(&request.domain) {
            if !existing.is_expired() && !existing.is_fenced() {
                return Err(LeaseError::Conflict(existing.lease_id));
            }
        }
        
        // Create new lease
        let lease = LeaseRecord::new(request);
        let proof = self.sign_lease(&lease);
        
        // Atomic update
        table.insert(request.domain, lease);
        Ok(proof)
    }
}
```

### Lease Verification

Before accepting any write, nodes must verify:

1. Lease covers the target path
2. Current time is within [start_ms, expiry_ms]
3. Signature validates against parent's public key
4. No fence certificate exists for this lease

## Fencing Protocol

When migrating leases or handling failures:

1. **Issue Fence Certificate**
   ```json
   {
     "type": "FENCE",
     "domain": "/data/users/",
     "fenced_lease_id": "london-1754842710000",
     "issuer": "europe-cg",
     "reason": "migration",
     "signature": "bls:0x2c8f..."
   }
   ```

2. **Propagate Fence**
   - Flood fence certificate to all CG members
   - Once fenced, lease holder must stop writes
   - New lease can only be issued after fence is confirmed

3. **Atomic Handoff**
   - New lease includes reference to fenced predecessor
   - Ensures clean transition with no overlap

## Integration with BFT

For cross-CG lease operations, use BFT consensus:

- Lease grants require 2f+1 signatures from parent CG
- Fence operations require BFT commit proof
- Ensures Byzantine nodes can't forge leases

## Security Properties

1. **Unforgeability**: Only authorized parent CG can issue valid leases
2. **Non-overlap**: Atomic lease table prevents concurrent leases
3. **Monotonicity**: Fencing ensures clean handoffs
4. **Auditability**: All lease operations carry cryptographic proofs
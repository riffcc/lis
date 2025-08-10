# CRDT Types and Merge Rules

## Overview

RHC uses Conflict-Free Replicated Data Types (CRDTs) to ensure deterministic conflict resolution during network partitions and asynchronous replication. Each data type has specific merge semantics that guarantee convergence.

## Core CRDT Types

### 1. OR-Set (Observed-Remove Set)

**Use Case**: User lists, tag sets, membership lists

**Structure**:
```json
{
  "type": "or-set",
  "adds": [
    {"elem": "alice", "id": "london#1", "timestamp": "1754842710000:0"},
    {"elem": "bob", "id": "london#1", "timestamp": "1754842710000:1"},
    {"elem": "charlie", "id": "london#2", "timestamp": "1754842711000:0"}
  ],
  "removes": [
    {"elem": "alice", "removed_id": "london#1", "timestamp": "1754842715000:0"}
  ]
}
```

**Merge Rule**:
- Element is in set if: ∃ add(elem, id) AND ∄ remove(elem, id)
- Concurrent adds create multiple instances
- Remove only affects specific add instance

**Example**:
```rust
fn merge_or_set(local: &ORSet, remote: &ORSet) -> ORSet {
    let mut merged = ORSet::new();
    
    // Union all adds
    merged.adds = union_by_id(&local.adds, &remote.adds);
    
    // Union all removes
    merged.removes = union_by_id(&local.removes, &remote.removes);
    
    merged
}
```

### 2. PN-Counter (Positive-Negative Counter)

**Use Case**: Account balances, quotas, metrics

**Structure**:
```json
{
  "type": "pn-counter",
  "increments": {
    "london": 150,
    "newyork": 75,
    "perth": 30
  },
  "decrements": {
    "london": 20,
    "newyork": 15,
    "perth": 0
  }
}
```

**Merge Rule**:
- Per-node increment/decrement tracking
- Value = Σ(increments) - Σ(decrements)
- Merge = pointwise max of each node's counts

**Example**:
```rust
fn merge_pn_counter(local: &PNCounter, remote: &PNCounter) -> PNCounter {
    let mut merged = PNCounter::new();
    
    // Take max of each node's increments
    for (node, &count) in local.increments.iter() {
        merged.increments.insert(
            node.clone(),
            count.max(*remote.increments.get(node).unwrap_or(&0))
        );
    }
    
    // Take max of each node's decrements
    for (node, &count) in local.decrements.iter() {
        merged.decrements.insert(
            node.clone(),
            count.max(*remote.decrements.get(node).unwrap_or(&0))
        );
    }
    
    merged
}
```

### 3. LWW-Register (Last-Write-Wins Register)

**Use Case**: Configuration values, single-value fields

**Structure**:
```json
{
  "type": "lww-register",
  "value": "config-value-123",
  "timestamp": "1754842715000:0",
  "writer": "london"
}
```

**Merge Rule**:
- Compare timestamps (HLC)
- Latest timestamp wins
- Tie-break by writer ID (deterministic ordering)

### 4. MV-Register (Multi-Value Register)

**Use Case**: Documents requiring conflict visibility

**Structure**:
```json
{
  "type": "mv-register",
  "values": [
    {
      "value": "version-A",
      "timestamp": "1754842715000:0",
      "writer": "london"
    },
    {
      "value": "version-B", 
      "timestamp": "1754842715000:0",
      "writer": "perth"
    }
  ]
}
```

**Merge Rule**:
- Keep all concurrent values
- Application chooses resolution strategy
- Explicit conflict visibility

### 5. RGA (Replicated Growable Array)

**Use Case**: Ordered lists, text editing

**Structure**:
```json
{
  "type": "rga",
  "elements": [
    {"id": "london#1", "value": "first", "after": "root"},
    {"id": "perth#1", "value": "second", "after": "london#1"},
    {"id": "newyork#1", "value": "third", "after": "london#1"}
  ],
  "tombstones": ["london#1"]
}
```

**Merge Rule**:
- Preserve insertion order via after-links
- Concurrent inserts at same position use ID ordering
- Tombstones mark deletions

## Deterministic Global Ordering

For recovery and replay, establish total order:

```rust
fn operation_order_key(op: &Operation) -> OrderKey {
    OrderKey {
        lease_creation_time: op.lease_proof.start_ms,
        lease_id: op.lease_proof.lease_id.clone(),
        local_op_timestamp: op.timestamp,
        op_hash: blake3::hash(&op.data),
    }
}

// Sort operations deterministically
operations.sort_by_key(|op| operation_order_key(op));
```

## Integration with RHC

### 1. Per-Path CRDT Selection

```yaml
/data/users.db:
  type: or-set
  merge: automatic

/data/balances/:
  type: pn-counter
  merge: automatic
  
/data/config.json:
  type: lww-register
  merge: automatic

/data/documents/:
  type: mv-register
  merge: manual  # App must resolve
```

### 2. Burst Buffer Integration

During partition:
1. Write to local burst buffer
2. Tag with CRDT type and metadata
3. On reconnect, merge via CRDT rules
4. Apply merged state atomically

### 3. Verification

Each CRDT operation includes:
- Type declaration
- Operation metadata (timestamp, writer)
- Causality information (vector clocks for MV-Register)
- Merge trace for auditability

## Example: Three-Way Merge

```rust
// Scenario 5 from cap_demo: London, NewYork, Perth merge
fn three_way_merge(london: &UserSet, newyork: &UserSet, perth: &UserSet) -> UserSet {
    // All three are OR-Sets
    let mut merged = ORSet::new();
    
    // Collect all unique adds
    for adds in [&london.adds, &newyork.adds, &perth.adds] {
        for add_op in adds {
            merged.adds.insert(add_op.clone());
        }
    }
    
    // Collect all removes
    for removes in [&london.removes, &newyork.removes, &perth.removes] {
        for remove_op in removes {
            merged.removes.insert(remove_op.clone());
        }
    }
    
    // Result: deterministic merge regardless of order
    merged.materialize() // {alice, bob, charlie, dave, eve, frank}
}
```

## Performance Considerations

| CRDT Type | Space Complexity | Merge Complexity |
|-----------|-----------------|------------------|
| OR-Set | O(adds + removes) | O(n log n) |
| PN-Counter | O(nodes) | O(nodes) |
| LWW-Register | O(1) | O(1) |
| MV-Register | O(concurrent values) | O(n log n) |
| RGA | O(elements + tombstones) | O(n log n) |

## Best Practices

1. **Choose the Right Type**: Match CRDT semantics to data semantics
2. **Garbage Collection**: Periodically clean tombstones after global sync
3. **Causality Tracking**: Use vector clocks when operation order matters
4. **Conflict Visibility**: Use MV-Register when conflicts need app resolution
5. **Deterministic Tie-Breaking**: Always use stable node IDs for ordering
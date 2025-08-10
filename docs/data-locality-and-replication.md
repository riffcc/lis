# Data Locality and Replication in Lis

## Core Concept: Leases Move, Not Data

In Lis, data is replicated across regions for availability and read performance. What "moves" during lease migration is the **write authority** - the right to be the source of truth for that data.

## Architecture

### Global Replication
```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Sydney    │     │   London    │     │    NYC      │
│             │     │             │     │             │
│ ┌─────────┐ │     │ ┌─────────┐ │     │ ┌─────────┐ │
│ │ Block A │ │     │ │ Block A │ │     │ │ Block A │ │
│ │ (copy)  │ │     │ │ (copy)  │ │     │ │ (copy)  │ │
│ └─────────┘ │     └─────────┘ │     │ └─────────┘ │
│             │                   │     │             │
│ Lease: ✓    │     Lease: ✗     │     │ Lease: ✗    │
│ (writer)    │     (reader)     │     │ (reader)    │
└─────────────┘     └─────────────┘     └─────────────┘
```

### What Actually Happens During "Migration"

1. **Before**: NYC has the lease for Block A
   - NYC: Can write (is source of truth)
   - Sydney: Has copy, can only read
   - London: Has copy, can only read

2. **Latency Detected**: Sydney doing many writes to Block A
   - Each write proxied through NYC (high latency)
   - System detects pattern

3. **Lease Transfer**: Authority moves to Sydney
   - Sydney: Now has write authority
   - NYC: Becomes read-only replica
   - London: Still read-only replica
   - **No data moved** - Sydney already had a copy!

## Dynamic Replication Factor

### PID Controller for Replica Count
Inspired by Peerbit's approach:

```rust
struct ReplicationController {
    target_availability: f64,      // e.g., 99.99%
    min_replicas: u32,            // e.g., 3
    max_replicas: u32,            // e.g., 7
    
    // PID controller state
    proportional_gain: f64,
    integral_gain: f64,
    derivative_gain: f64,
    integral_error: f64,
    last_error: f64,
}

impl ReplicationController {
    fn calculate_replica_count(&mut self, current_availability: f64) -> u32 {
        let error = self.target_availability - current_availability;
        
        // PID calculation
        let p = error * self.proportional_gain;
        let i = self.integral_error * self.integral_gain;
        let d = (error - self.last_error) * self.derivative_gain;
        
        let adjustment = p + i + d;
        let new_count = (self.min_replicas as f64 + adjustment).round() as u32;
        
        new_count.clamp(self.min_replicas, self.max_replicas)
    }
}
```

### Factors Affecting Replication
- **Access patterns**: More replicas near frequent readers
- **Failure rates**: More replicas in unreliable regions
- **Cost constraints**: Fewer replicas for cold data
- **Regulatory**: Minimum replicas in specific jurisdictions

## VM-Specific Locality

### The Ultimate Goal
When a VM runs on a specific Proxmox host, we want:

1. **VM's hot blocks**: Lease holder is the local CG on that host
2. **VM's warm blocks**: Recent copy exists locally
3. **VM's cold blocks**: Can be remote (fetched on demand)

### Implementation Strategy

```rust
struct VMLocalityTracker {
    vm_id: VmId,
    current_host: NodeId,
    block_heat_map: HashMap<BlockId, BlockHeat>,
}

struct BlockHeat {
    read_count: u64,
    write_count: u64,
    last_access: HLCTimestamp,
    predicted_next_access: Option<HLCTimestamp>,
}

impl VMLocalityTracker {
    fn optimize_placement(&self) -> Vec<PlacementAction> {
        let mut actions = Vec::new();
        
        for (block_id, heat) in &self.block_heat_map {
            if heat.write_count > threshold {
                // Hot block - need local lease
                actions.push(PlacementAction::RequestLease {
                    block_id: *block_id,
                    target_node: self.current_host,
                });
            }
            
            if heat.read_count > threshold {
                // Warm block - need local replica
                actions.push(PlacementAction::EnsureReplica {
                    block_id: *block_id,
                    target_node: self.current_host,
                });
            }
        }
        
        actions
    }
}
```

## Metadata Layer (Hive) Role

The global metadata layer tracks:
- **Block → Lease Holder** mapping
- **Block → Replica Locations** list
- **Block → Version** for consistency
- **VM → Host** placement

This allows instant knowledge of:
- Where to send writes (lease holder)
- Where to read from (nearest replica)
- Whether local cache is stale

## Optimization Strategies

### 1. Predictive Pre-warming
```rust
// When VM migrates to new host
fn on_vm_migration(vm_id: VmId, new_host: NodeId) {
    // Pre-fetch hot blocks
    let hot_blocks = get_hot_blocks(vm_id);
    for block in hot_blocks {
        ensure_local_replica(new_host, block);
        if is_write_hot(block) {
            request_lease_transfer(new_host, block);
        }
    }
}
```

### 2. Affinity Groups
```rust
// Blocks that are accessed together
struct AffinityGroup {
    blocks: Vec<BlockId>,
    access_correlation: f64,
}

// When one block is accessed, pre-fetch its affinity group
fn on_block_access(block: BlockId) {
    if let Some(group) = get_affinity_group(block) {
        for related_block in group.blocks {
            prefetch_to_local_cache(related_block);
        }
    }
}
```

### 3. Time-based Patterns
```rust
// "This VM is used 9-5 Sydney time"
struct AccessSchedule {
    vm_id: VmId,
    active_hours: Vec<(TimeRange, Location)>,
}

// Pre-position leases before work hours
fn scheduled_lease_migration(schedule: &AccessSchedule) {
    let next_active = schedule.next_active_period();
    if let Some((time, location)) = next_active {
        schedule_task(time - 30.minutes(), || {
            migrate_vm_leases(schedule.vm_id, location);
        });
    }
}
```

## Real-World Example

### Scenario: Global Development Team

1. **Initial State**:
   - VM hosted in NYC datacenter
   - Git repository blocks replicated globally
   - All leases held by NYC

2. **Developer in Sydney starts work**:
   - First `git pull`: Reads from Sydney replica (fast)
   - First edit: Write proxied to NYC (slow)
   - After 3 edits: Lease transfers to Sydney
   - Subsequent edits: Local writes (fast)

3. **Developer commits and pushes**:
   - Hot blocks (modified files): Sydney has lease
   - Cold blocks (untouched files): NYC still has lease
   - Push operation: Only hot blocks need consensus

4. **London developer starts work**:
   - `git pull`: Reads from London replica
   - Starts editing different files
   - Those blocks' leases migrate to London
   - Sydney keeps leases for their active files

5. **Result**:
   - Each developer has local write speed for their files
   - No manual configuration needed
   - Optimal resource usage (leases follow activity)

## Performance Implications

### Write Performance
- **With local lease**: 0.1ms (local SSD)
- **With remote lease**: Network RTT + 0.1ms
- **After lease migration**: Back to 0.1ms

### Read Performance
- **Always fast**: Reads from local replica
- **Cache coherence**: Metadata layer ensures freshness
- **No proxy needed**: Direct local reads

### Storage Efficiency
- **Hot data**: Multiple replicas, local leases
- **Warm data**: Fewer replicas, remote leases
- **Cold data**: Minimum replicas, archive tier

## Configuration

```toml
[replication]
# Minimum replicas for durability
min_replicas = 3

# Maximum replicas (cost control)
max_replicas = 7

# Target availability SLA
target_availability = 0.9999

# PID controller tuning
pid_proportional = 0.5
pid_integral = 0.1
pid_derivative = 0.05

[locality]
# Writes before attempting lease steal
lease_steal_threshold = 3

# Time before marking block as cold
cold_threshold_minutes = 60

# Prefetch affinity groups
enable_affinity_prefetch = true

# Schedule-based pre-warming
enable_scheduled_migration = true
```

## Summary

The magic of Lis isn't moving data around the globe - it's intelligently managing:
1. **Where write authority lives** (lease placement)
2. **How many copies exist** (replication factor)
3. **Which copies are fresh** (metadata consistency)
4. **What to cache locally** (heat tracking)

This creates the illusion of data following you, when really you're just getting intelligent lease assignment and ensuring the data you need is already in your local cache when you need it.
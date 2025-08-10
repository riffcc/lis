# Block-Level Lease Optimization

## Key Insight: Block-Level Conflicts Are Rare

When operating at the block level (e.g., 4KB blocks), most workloads naturally avoid conflicts:
- Different users edit different files → different blocks
- Even in the same file, users often edit different sections → different blocks
- Databases segment by table/index → different blocks
- VMs have distinct working sets → different blocks

This means we can optimize for the common case: **non-conflicting access patterns**.

## Multi-User Optimization Strategy

### The Goal
Minimize average write latency across ALL users of a block, not just the most active one.

### Latency Cost Function
```rust
struct BlockLeaseOptimizer {
    block_id: BlockId,
    access_history: Vec<AccessRecord>,
}

struct AccessRecord {
    user_location: Location,
    timestamp: HLCTimestamp,
    latency_ms: u64,
    access_type: AccessType,
}

impl BlockLeaseOptimizer {
    /// Calculate optimal lease placement for minimum average latency
    fn calculate_optimal_placement(&self) -> Location {
        let locations = self.get_unique_locations();
        let mut best_location = locations[0].clone();
        let mut best_avg_latency = f64::MAX;
        
        // Try each potential lease location
        for candidate_location in &locations {
            let avg_latency = self.calculate_average_latency_if_placed_at(candidate_location);
            
            if avg_latency < best_avg_latency {
                best_avg_latency = avg_latency;
                best_location = candidate_location.clone();
            }
        }
        
        best_location
    }
    
    /// Calculate average latency if lease were at given location
    fn calculate_average_latency_if_placed_at(&self, lease_location: &Location) -> f64 {
        let recent_accesses = self.get_recent_accesses(Duration::from_secs(300)); // Last 5 min
        
        let total_latency: u64 = recent_accesses.iter()
            .map(|access| {
                if access.user_location == *lease_location {
                    1 // Local write latency
                } else {
                    self.network_latency(&access.user_location, lease_location) * 2 + 1
                }
            })
            .sum();
            
        total_latency as f64 / recent_accesses.len() as f64
    }
}
```

## Adaptive Threshold Based on Contention

### Low Contention (Common Case)
When only one user is accessing a block:
```rust
fn steal_threshold_single_user(latency_factor: f64) -> u64 {
    match latency_factor as u64 {
        50.. => 1,   // 50x slower? Steal immediately
        10.. => 2,   // 10x slower? 2 writes
        5..  => 3,   // 5x slower? 3 writes
        2..  => 5,   // 2x slower? 5 writes
        _    => 10,  // Otherwise need sustained access
    }
}
```

### High Contention (Rare Case)
When multiple users are actively writing to the same block:
```rust
fn steal_threshold_multi_user(
    my_latency: u64,
    current_avg_latency: f64,
    potential_new_avg: f64
) -> bool {
    // Only steal if it improves OVERALL experience
    let improvement_ratio = current_avg_latency / potential_new_avg;
    
    // Require significant improvement to avoid thrashing
    improvement_ratio > 1.5 && my_latency > 100 // At least 50% better for all
}
```

## Block Access Patterns in Practice

### Example 1: Shared Git Repository
```
Developer A (Sydney):  Edits src/auth.rs     → Blocks 1000-1050
Developer B (London):  Edits src/network.rs  → Blocks 2000-2100  
Developer C (NYC):     Edits tests/api.rs    → Blocks 3000-3200

Result: Each developer gets local leases for their blocks. No conflicts!
```

### Example 2: Shared Database
```
Region A: INSERT INTO users_asia    → Blocks 5000-5999 (asia partition)
Region B: INSERT INTO users_europe  → Blocks 6000-6999 (europe partition)
Region C: UPDATE inventory WHERE... → Blocks 7000-7010 (specific rows)

Result: Natural partitioning means each region works on different blocks.
```

### Example 3: Actual Conflict - Shared Configuration File
```
Admin A (Tokyo):   Edits config.yaml line 10   → Block 100
Admin B (Berlin):  Edits config.yaml line 12   → Block 100 (same block!)
Admin C (Toronto): Edits config.yaml line 15   → Block 100 (same block!)

This is where multi-user optimization kicks in:
- Calculate center of mass of access
- Place lease to minimize total latency
- Maybe ends up in London (middle point)
```

## Fairness and Anti-Starvation

### Preventing Lease Ping-Pong
```rust
struct LeaseStealGovernor {
    last_steal_time: HashMap<BlockId, HLCTimestamp>,
    steal_cooldown: Duration,
}

impl LeaseStealGovernor {
    fn can_steal(&self, block_id: BlockId, now: HLCTimestamp) -> bool {
        if let Some(last_steal) = self.last_steal_time.get(&block_id) {
            now.duration_since(last_steal) > self.steal_cooldown
        } else {
            true // Never stolen before
        }
    }
}
```

### Weighted History
Recent accesses count more than old ones:
```rust
fn calculate_weighted_average_latency(&self, lease_location: &Location) -> f64 {
    let now = HLCTimestamp::now();
    let mut weighted_sum = 0.0;
    let mut weight_total = 0.0;
    
    for access in &self.access_history {
        let age = now.duration_since(access.timestamp);
        let weight = (-age.as_secs() as f64 / 300.0).exp(); // Exponential decay
        
        let latency = self.calculate_latency(&access.user_location, lease_location);
        weighted_sum += latency as f64 * weight;
        weight_total += weight;
    }
    
    weighted_sum / weight_total
}
```

## Optimization for Common Patterns

### 1. Follow-the-Sun Development
```rust
// Detect time-based patterns
fn detect_timezone_pattern(&self) -> Option<TimezonePattern> {
    // Analyze access times vs locations
    // Return predicted active timezone for next period
}

// Pre-position leases before workday starts
fn schedule_proactive_migration(&self, pattern: TimezonePattern) {
    let next_active_region = pattern.next_active_region();
    let migration_time = pattern.workday_start() - Duration::from_mins(15);
    
    schedule_at(migration_time, || {
        self.migrate_leases_to(next_active_region);
    });
}
```

### 2. Paired Programming
```rust
// Detect correlated access
fn detect_collaboration(&self) -> Vec<CollaborationGroup> {
    // Find users who access same blocks within short time windows
    // Place leases at geographic center of group
}
```

### 3. Batch Processing
```rust
// Detect sequential access patterns
fn detect_batch_pattern(&self) -> Option<BatchPattern> {
    // If blocks accessed sequentially, pre-fetch leases
    // Optimize for throughput over latency
}
```

## Real-World Example: Global Software Team

### Morning: Asia-Pacific Active
```
Block 1000 (auth.rs): Sydney has lease (1ms writes)
Block 2000 (api.rs):  Singapore has lease (1ms writes)  
Block 3000 (db.rs):   Tokyo has lease (1ms writes)
Block 4000 (ui.js):   No recent access (lease in cheapest location)
```

### Afternoon: Europe Comes Online
```
Block 1000: Sydney still has lease (only Sydney editing)
Block 2000: London steals lease (London now active, Singapore idle)
Block 3000: Tokyo keeps lease (still actively editing)
Block 4000: Berlin gets lease (starts working on UI)
```

### Evening: Americas Join
```
Block 1000: Sydney done → NYC steals lease for new feature
Block 2000: London keeps (still active)
Block 3000: SF steals lease (Tokyo went home)
Block 4000: Berlin/NYC share → lease in Toronto (midpoint)
```

## Configuration

```toml
[lease_optimization]
# Minimum improvement ratio for multi-user blocks
min_improvement_ratio = 1.5

# Cooldown between steal attempts
steal_cooldown_ms = 30000

# History window for access patterns
history_window_secs = 300

# Enable predictive positioning
predictive_positioning = true

# Block access correlation threshold
correlation_threshold = 0.8

[fairness]
# Maximum consecutive leases from one location
max_consecutive_ownership = 100

# Starvation prevention timeout
starvation_timeout_ms = 60000
```

## Performance Impact

### Single User Per Block (90%+ of cases)
- Immediate lease migration on sustained access
- Near-zero coordination overhead
- Optimal placement within 3-5 writes

### Multiple Users Per Block (<10% of cases)
- Intelligent placement minimizing average latency
- Cooldown prevents thrashing
- Fair access via weighted history

### System-Wide
- Reduced WAN traffic (fewer proxied writes)
- Better cache locality (related blocks cluster)
- Lower metadata churn (stable lease placement)

## Conclusion

By optimizing at the block level and recognizing that conflicts are rare, Lis can provide:
1. **Local write performance** for most users most of the time
2. **Fair sharing** when conflicts do occur
3. **Predictive optimization** based on patterns
4. **Minimal coordination overhead** 

The result: a globally distributed system that feels local to everyone, because at the block level, everyone is usually working on different data!
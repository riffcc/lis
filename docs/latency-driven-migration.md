# Latency-Driven Lease Migration

## Overview

Lis automatically migrates leases to where data is being actively used, inspired by Bunny.net's edge container strategy. When write latency exceeds configurable thresholds, consensus groups automatically attempt to steal leases, bringing data control closer to active users.

## Core Concept

"Data doesn't live in a place, it lives where it's being USED."

Instead of manually configuring data placement, Lis observes actual usage patterns and automatically optimizes. If you're consistently writing from Singapore to a lease holder in London, Singapore will eventually steal the lease.

## Migration Triggers

### Dynamic Thresholds
```rust
fn should_attempt_steal(local_latency: Duration, remote_latency: Duration, write_count: u64) -> bool {
    let latency_factor = remote_latency.as_millis() / local_latency.as_millis();
    
    match latency_factor {
        10.. => write_count >= 3,      // 10x slower? Steal after 3 writes
        5..10 => write_count >= 5,     // 5x slower? Steal after 5 writes  
        2..5 => write_count >= 10,     // 2x slower? Steal after 10 writes
        _ => false,                    // Less than 2x? Not worth migrating
    }
}
```

### Write Pattern Tracking
Each consensus group maintains:
- Recent write latencies per lease scope
- Write frequency from each geographic region
- Historical access patterns for prediction

## Block-Level Granularity

With global metadata consistency (Hive), we can migrate at block granularity:

### Hot Block Detection
```rust
struct BlockHeatMap {
    block_id: BlockId,
    write_count: u64,
    last_write: HLCTimestamp,
    accessing_regions: HashMap<Region, u64>,
}
```

### Selective Migration
1. Track which blocks are being written frequently
2. Migrate ONLY hot blocks to the active region
3. Cold blocks remain where they are
4. VM appears omnipresent but hot data follows usage

### Example Scenario
- VM hosted in NYC
- Student in Sydney starts using it
- Initially: All blocks in NYC, 200ms write latency
- After 5 writes to document blocks: Those blocks migrate to Sydney
- Result: Document editing at local speeds, OS blocks stay in NYC

## Predictive Pre-Migration

### Access Pattern Learning
```rust
struct AccessPattern {
    user: UserId,
    correlated_blocks: Vec<BlockId>,
    time_of_day_pattern: Option<TimePattern>,
    geographic_pattern: Option<GeoPattern>,
}
```

### Speculative Migration
- "User always accesses these 10 files together"
- When they touch one file, pre-migrate the others
- "User works from Sydney 9am-5pm, London evenings"
- Pre-migrate workspace before they log in

## Implementation Strategy

### Phase 1: Latency Monitoring
- Add latency tracking to all write operations
- Build heat maps of block access patterns
- Identify migration candidates

### Phase 2: Basic Migration
- Implement lease stealing based on latency
- Start with file-level granularity
- Measure improvement in write latency

### Phase 3: Smart Migration
- Block-level granularity
- Predictive pre-migration
- Cost-aware migration (don't migrate for one-off access)

### Phase 4: Global Optimization
- Cross-VM correlation (users often access multiple VMs)
- Network topology awareness (prefer migrations along fast links)
- Storage tier awareness (hot data on SSD, cold on HDD)

## Configuration

```toml
[migration]
# Minimum latency factor to consider migration
min_latency_factor = 2.0

# Number of slow writes before attempting steal
slow_write_threshold = 5

# Minimum time between migration attempts
migration_cooldown_ms = 30000

# Maximum blocks to migrate at once
max_concurrent_migrations = 100

# Enable predictive pre-migration
predictive_migration = true
```

## Proxmox Global Cluster Plugin

### Architecture
```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Proxmox NYC   │     │ Proxmox London  │     │ Proxmox Sydney  │
│                 │     │                 │     │                 │
│  ┌───────────┐  │     │  ┌───────────┐  │     │  ┌───────────┐  │
│  │ Lis Plugin│  │◄────┤  │ Lis Plugin│  │◄────┤  │ Lis Plugin│  │
│  └───────────┘  │     │  └───────────┘  │     │  └───────────┘  │
│        │        │     │        │        │     │        │        │
│  ┌─────▼─────┐  │     │  ┌─────▼─────┐  │     │  ┌─────▼─────┐  │
│  │   Hive    │◄─┼─────┼─▶│   Hive    │◄─┼─────┼─▶│   Hive    │  │
│  │ Metadata  │  │     │  │ Metadata  │  │     │  │ Metadata  │  │
│  └───────────┘  │     │  └───────────┘  │     │  └───────────┘  │
└─────────────────┘     └─────────────────┘     └─────────────────┘
         ▲                       ▲                       ▲
         └───────────────────────┴───────────────────────┘
                     Vesper Mesh (Auto-discovery)
```

### Installation
```bash
# On each Proxmox node
apt install proxmox-lis-global
systemctl enable lis-cluster
lis-cluster join --auto-discover
```

### Features
- **Auto-discovery**: Nodes find each other via Vesper
- **Transparent migration**: VMs don't know they're moving
- **Unified UI**: Single pane of glass for global infrastructure
- **Usage analytics**: See where your VMs are actually being used
- **Cost optimization**: Automatically consolidate during low usage

## Performance Characteristics

### Latency Improvements
- Local region: 0.1ms → 0.1ms (no change)
- Same continent: 50ms → 0.1ms (500x improvement)
- Cross-globe: 200ms → 0.1ms (2000x improvement)

### Migration Overhead
- Block migration: ~100ms + transfer time
- Lease steal: ~50ms (consensus round)
- Metadata update: ~10ms (Hive operation)

### Steady State
After migrations settle:
- Hot data: Local write speeds everywhere
- Cold data: No migration overhead
- Network usage: Minimal (only active blocks move)

## Use Cases

### Global Development Teams
- Developers in multiple timezones
- Code follows the active developer
- No more "waiting for the US to wake up"

### Follow-the-Sun Operations
- Support teams hand off VMs
- Data migrates with the active team
- Always local performance

### Edge Computing
- IoT data processing
- Data stays near the sensors
- Aggregations migrate to analysis centers

### Disaster Recovery
- Automatic migration away from failing regions
- No manual intervention required
- Data flows to healthy nodes

## Future Enhancements

### AI-Driven Optimization
- Learn organization patterns
- Predict migrations before needed
- Optimize for cost vs performance

### Multi-Cloud Federation
- Span AWS, GCP, Azure, on-prem
- Transparent data movement
- Regulatory compliance awareness

### Application Awareness
- Understand application access patterns
- Co-locate related data
- Database-aware optimizations

## Conclusion

Latency-driven migration transforms Lis from a distributed filesystem into an intelligent data platform that automatically optimizes itself. Combined with Proxmox integration, it becomes the foundation for truly global infrastructure that "just works."

No more manual migration. No more geographic limitations. Data lives where it's used, automatically.
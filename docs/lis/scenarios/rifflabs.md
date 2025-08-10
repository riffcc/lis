# RiffLabs: A Multi-Site Storage Federation Scenario

## Overview

RiffLabs operates four geographically distributed labs with varying storage capacities and performance characteristics. This scenario explores how Lis with RHC can unify these resources into a single, globally distributed storage system with local-first performance.

## Infrastructure

### Perth Lab (Primary)
- **Capacity**: 2.4PB
- **Compute**: 864 EPYC cores
- **Storage Mix**: Primarily HDD with some NVMe
- **Role**: Bulk storage, archival, compute-intensive workloads

### London Lab (Main)
- **Capacity**: 160TB
- **Storage Mix**: Balanced HDD/SSD
- **Role**: Primary operations, frequently accessed data

### London MiniLab
- **Capacity**: 16TB
- **Storage**: Pure NVMe
- **Role**: Hot data, database storage, low-latency applications

### London Secondary Lab
- **Capacity**: 45TB
- **Storage Mix**: HDD/SSD blend
- **Role**: Development, testing, overflow capacity

## Networking

All sites connected via Yggdrasil Network, providing:
- Encrypted mesh networking
- Stable addressing across NAT
- Direct peer-to-peer connectivity where possible
- Automatic routing around failures

## Storage Unification Goals

### 1. Local-First Performance

**Requirement**: Applications should experience storage latency equivalent to local disk access.

**RHC Solution**:
- Each node holds read replicas of frequently accessed data
- Write leases automatically migrate to active usage locations
- Chunkservers with NVMe can ACK writes immediately before replication

### 2. Flexible Consistency Models

**Per-File Configuration Options**:

```
/data/critical/database.db
  - consistency: strong
  - write_ack: 3 nodes same rack
  - storage_class: nvme_only
  
/data/media/movies/
  - consistency: eventual  
  - write_ack: 1 node local
  - storage_class: any
  - replication: 2 copies globally

/data/backups/
  - consistency: eventual
  - write_ack: 1 node local  
  - storage_class: hdd_preferred
  - replication: perth=2,london=1
```

### 3. Tiered Write Acknowledgment

**Fast ACK Mode** (for development/testing):
- ACK after local NVMe write
- Background replication to meet goals
- Risk: Data loss if node fails before replication

**Balanced Mode** (default):
- ACK after N nodes in same rack confirm
- Configurable N (typically 2-3)
- Good balance of performance and durability

**Safe Mode** (for critical data):
- ACK after geographic distribution met
- Example: 1 copy in Perth, 2 in London
- Higher latency but maximum durability

### 4. Storage Locality Preferences

**Rack-Aware Placement**:
```
write_placement:
  - prefer: same_node      # Try local first
  - then: same_rack        # Same rack if local full
  - then: same_site        # Same geographic site
  - finally: any           # Any available node
```

**NVMe Burst Buffer**:
- All writes initially go to nearest NVMe storage
- Background migration to HDD based on access patterns
- Hot data remains on NVMe, cold data moves to HDD

## RHC Consensus Groups

### Hierarchy

```
Global CG
├── Perth CG
│   └── Perth-Rack-* CGs
└── London CG
    ├── London-Main CG
    ├── London-Mini CG
    └── London-Secondary CG
```

### Lease Management

**Write Patterns**:
- VM disk images: Lease follows VM migration
- Shared datasets: Lease with whoever's actively writing
- Archival data: Lease stays in Perth (bulk storage)

**Automatic Lease Migration**:
- Monitor write patterns over 5-minute windows
- If London generates >80% of writes to a file, migrate lease
- Pre-committed approvals enable instant handoff

## Multi-Protocol Access

### POSIX Filesystem
- Mounted at each site via FUSE
- Full POSIX semantics with distributed locking
- Appears as single global namespace

### S3-Compatible Object Storage
- RESTful API gateway at each site
- Bucket policies map to RHC leases
- Eventually consistent by default, tunable per bucket

### Block Storage
- iSCSI/NBD export for VM disks
- Strong consistency required
- Leases follow VM live migration

## Operational Scenarios

### Scenario 1: VM Migration
1. VM running in Perth needs to move to London
2. Block device lease transfers with VM
3. Recent blocks already cached in London (read-ahead)
4. Post-migration writes stay local to London

### Scenario 2: Development Workflow
1. Developer in London works on large dataset
2. First read pulls data from Perth
3. Subsequent reads served from London cache
4. Writes go to London MiniLab NVMe first
5. Background replication to Perth for durability

### Scenario 3: Backup Operation
1. Nightly backups written locally first
2. Each site backs up to local storage
3. Cross-site replication during off-hours
4. Perth maintains 2 copies, London keeps 1

### Scenario 4: Database Hosting
1. Database files on London MiniLab NVMe
2. Synchronous replication to London Main
3. Async replication to Perth
4. Lease pinned to London for low latency

## Configuration Examples

### Global Settings
```yaml
rhc:
  lease_duration: 30s
  hlc_max_drift: 60s
  
storage:
  default_replication: 3
  min_copies_per_site: 1
  
network:
  mesh: yggdrasil
  encryption: required
```

### Per-Directory Rules
```yaml
/data/vms/:
  lease_affinity: follow_vm
  consistency: strong
  storage_class: ssd_preferred
  
/data/archive/:
  lease_affinity: perth
  consistency: eventual  
  storage_class: hdd_only
  compression: enabled
```

## Performance Expectations

### Local Operations
- Read latency: ~0.1ms (memory cache)
- Write latency: 0.5-5ms (depending on ACK policy)
- Throughput: Full NVMe/SSD speed when local

### Cross-Site Operations
- Perth ↔ London: ~250ms RTT
- Lease transfer: <300ms typically
- Background replication: Shaped to available bandwidth

## Benefits

1. **Unified Namespace**: Single storage system across all sites
2. **Optimal Performance**: Local speeds for active data
3. **Flexible Consistency**: Tunable per-workload
4. **Automatic Optimization**: Leases and data follow usage
5. **Multi-Protocol**: POSIX, S3, and block storage from same pool
6. **Hardware Efficiency**: Use NVMe for hot data, HDD for cold

This architecture allows RiffLabs to operate as if all storage is local while maintaining global durability and accessibility.
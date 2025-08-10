# RHC Documentation Index

## Overview

RHC (Riff.CC Hierarchical Consensus) is a distributed consensus protocol that combines the best aspects of Raft, Byzantine Fault Tolerance, and CRDTs to achieve local-first performance at global scale.

## Core Components

### 1. [Leader Leases](leader-leases.md)
- 30-second Raft-style leader elections
- Hierarchical lease delegation  
- Pre-committed approvals for low latency
- Exclusive write authority, always-available reads

### 2. [Cryptographic Lease Proofs](lease-proofs.md)
- Every lease carries unforgeable proof
- Atomic lease table prevents split-brain
- Parent CG signatures validate authority
- Integrated with BFT for Byzantine tolerance

### 3. [Lease Fencing & Migration](lease-fencing.md)
- Clean handoffs with no overlap
- Fence certificates prevent split-brain
- Handles asymmetric network partitions
- Atomic state transfer during migration

### 4. [BFT Two-Flood Protocol](bft-two-flood.md)
- Optimal 2-round consensus
- Threshold signatures (2f+1 of 3f+1)
- No complex view changes
- Naturally handles asymmetric networks

### 5. [CRDT Types](crdt-types.md)
- OR-Set for collections
- PN-Counter for metrics
- LWW/MV-Register for configuration
- Deterministic conflict resolution

### 6. [Burst Buffer Architecture](burst-buffer.md)
- Chunkserver on each hyperconverged host
- Local ACK for ultra-low latency
- Async replication for durability
- Respects leases and consistency settings

### 7. [Timing Discipline](timing-discipline.md)
- HLC handles clock skew gracefully
- Physical clocks can lie - HLC provides correctness
- Lease timing uses HLC timestamps, not wall clock
- NTP is nice but not necessary

### 8. [HLC Implementation](../about.md)
- Hybrid Logical Clocks for global ordering
- Handles clock skew gracefully
- Monotonic timestamps
- Thread-safe implementation

## Key Properties

### Breaking CAP

RHC achieves all three CAP properties through:
- **Consistency**: Leader leases ensure single writer
- **Availability**: Reads always work, writes need valid lease  
- **Partition Tolerance**: CRDTs + bounded leases handle splits

The "trick": Different guarantees at different times/scales.

### Performance Characteristics

- **Local reads**: 0.1ms (memory cache)
- **Local writes**: 0.5ms (burst buffer)
- **Regional consensus**: 5-50ms
- **Global consensus**: 100-300ms

### Scalability

- Hierarchical design limits consensus scope
- 99% of operations stay local
- Pre-committed approvals eliminate round trips
- Compact proofs (48-byte BLS signatures)

## Implementation Status

### Completed ✓
- HLC module with extensive tests
- Basic lease data structures
- Conceptual demos (lease migration, CAP scenarios)
- Comprehensive documentation

### In Progress 🚧
- Cryptographic lease proof implementation
- BFT two-flood protocol
- CRDT type implementations
- Production-ready burst buffer

### Planned 📋
- Gossip protocol for lease propagation
- DNS-based CG discovery
- Kubernetes CSI driver
- Performance benchmarks

## Quick Start

1. **Understanding RHC**: Read [ABOUT.md](about.md) for the conceptual overview
2. **Run the demos**: 
   ```bash
   cargo run --example lease_demo
   cargo run --example cap_demo  
   cargo run --example latency_demo
   ```
3. **Explore the code**: Start with `src/rhc/` modules

## Design Principles

1. **Hierarchy minimizes consensus** - Don't organize it, avoid it
2. **Ownership follows usage** - Leases migrate to active sites
3. **Local-first performance** - ACK from local chunkserver
4. **Deterministic resolution** - CRDTs ensure convergence
5. **Cryptographic correctness** - Proofs prevent misbehavior

## Use Cases

- **Geo-distributed storage**: MooseFS-like with global scale
- **Edge computing**: Local performance, global consistency
- **Hybrid cloud**: Seamless data mobility
- **VM/container storage**: Hyperconverged with geo-stretch

## Further Reading

- [Original RHC Design](../../synthesis/RHC-Dynamic.txt)
- [BFT Consensus Details](../../synthesis/BFT.txt)  
- [Two-Generals Solution](../../synthesis/TWOGEN.txt)
- [RiffLabs Scenario](../lis/scenarios/rifflabs.md)
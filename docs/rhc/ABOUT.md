# RHC: Riff.CC Hierarchical Consensus
## A Multi-Layer Consensus Protocol for Planet-Scale Distributed Systems

## Abstract

We present Riff.CC Hierarchical Consensus (RHC), a novel consensus protocol that combines recursive Raft leader leases with Byzantine Fault Tolerant (BFT) flooding to achieve microsecond local consistency while maintaining global convergence. RHC transcends traditional CAP theorem limitations by operating at multiple temporal and geographic scales simultaneously, enabling planet-scale distributed systems with local-first performance.

### Key components
* Raft-style leader elections
  * Per file-folder.
* Global namespace, inspired by MooseFS
  * Configurable options per file and folder.
* Hybrid Logical Clocks for ordering events
* Conflict-Free Replicated Data Types (CRDTs) for conflict resolution
* Distributed DNS for service discovery and load balancing, service registration.

## 1. Introduction

Traditional distributed consensus protocols force a choice: either strong consistency with global coordination overhead, or eventual consistency with complex conflict resolution. RHC eliminates this false dichotomy by introducing hierarchical consensus layers that operate at their natural speeds.

### 1.1 Key Innovations

- **Recursive Leader Leases**: Hierarchical Raft leaders with bounded authority domains
- **Threshold Signature Aggregation**: BLS-based compact proofs for global coordination
- **Temporal Hierarchy**: Different consistency guarantees at different geographic scales
- **Flood-Based Propagation**: Network-partition resilient message delivery

### 2. System Architecture

### 2.1 Hierarchy Levels

RHC operates using *flexible* hierarchy.

This means a Consensus Group (CG) can be formed at any granularity, allowing for efficient coordination at various scales.

### 2.2 Consensus Groups (CGs)

Consensus Groups (CGs) are the fundamental building blocks of RHC's hierarchy. Each CG is a group of nodes that work together to maintain a consistent view of the data. CGs can be formed at any granularity, allowing for efficient coordination at various scales.

Examples of CGs might include:
- A group of nodes in a bedroom, and a group of nodes in an office
- A building which contains both of those groups
- A city which contains multiple buildings
- A country which contains multiple cities
- A continent which contains multiple countries
- The global CG, containing all CGs.

CGs do not have a strict hierarchy - they can overlap, intersect, or be nested within each other. They can also be dynamically created and destroyed as needed. CGs can be statically configured (similar to MooseFS' rack topology and network classes), discovered via DNS, or dynamically created based on network topology or even latency.

### 2.3 Subscriptions and Membership
#### DNS-based CGs (Discovery CGs)
Membership can be defined in DNS - using DNS SRV records to advertise the presence of a CG, and DNS TXT records to provide metadata about the CG and its memberships.

#### Static CGs
CGs can be statically configured (similar to MooseFS' rack topology and network classes), allowing for predefined and controlled membership.

#### Managed CGs
Nodes can be part of CGs by joining or leaving them, but only with the permission of participants in CGs above and below a particular CG.

This allows for nodes in London to automatically join and discover their relevant CGs, without being able to interfere with CGs that represent other regions of the world.

### 2.4 Leader Leases
Leader leases are used to ensure strong consistency over a specific section of the filesystem. They are time-bounded and can be automatically renewed via heartbeat. In case of a failure, the lease can be quickly handed off to another node via the expiration of a specific lease.

#### 2.4.1 Lease Management
**Lease Management**:
- Leases are time-bounded: `Lease(domain, start_time, duration, holder)`
- Automatic renewal via heartbeat
- Instant handoff via signed transfer proof

### 3. Scenarios

**Scenario A**:

A node in Perth wants to write to a file normally located in London.

If the file is actively in use in London, a leader lease will exist for it, ensuring only the London region has claim to that specific section of the filesystem. The node in Perth can request the leader lease, and if successful, it can proceed with writing to the file.

If the file is not actively in use in London, the Perth node can simply claim it via a leader lease. Once elected, Perth has strong consistency over that file - no other region can commit writes, and any other region reading it will see eventually consistent changes flow through from Perth to it.

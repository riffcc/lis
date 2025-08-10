# lis
Life is short, but data should live forever

## the world computer
Lis is an experimental distributed filesystem intended to stretch the boundaries of CAP theorem and provide a unified view of data across multiple nodes, anywhere in the world.

We use Riff Hierarchical Consensus (RHC) to achieve strong consistency and availability while tolerating network partitions.

### Key Innovation: Intelligent Lease Placement
Unlike traditional distributed filesystems where data has a fixed "home", Lis dynamically moves write authority (leases) to where data is being actively used. Data is replicated globally for reads, but writes happen locally through intelligent lease migration. This creates the illusion of data following you while maintaining strong consistency.

You can read more about RHC in `docs/` directory.

## Planned Features
Performance:
* Node affinity for local writes, burst-buffer style


Networking:
* Fully IPv6 native with support for IPv4
* Fully georeplicated, with support for multi-tenancy, multi-region and even multi-organization clusters

Practicality:
* Strong self-hosting support
* Self-healing and data safety
* Fast erasure coding
* Integration with Yggdrasil for effortless NAT traversal

Security:
* Strong encryption and authentication

## Installation
* Install Rust via Rustup: https://rustup.rs/
* Clone the repository: `git clone https://github.com/riffcc/lis.git && cd lis`
* Build and run the project: `cargo run`

## Development
* Run tests: `cargo test`
* Run benchmarks: `cargo bench`
* Run linting: `cargo clippy`
* Deploy a test cluster: `make test-cluster`

## Demos and Examples

The `examples/` directory contains demonstrations of key RHC concepts:

### Core Concepts
* `basic_lease_operations` - Demonstrates fundamental lease operations: request, grant, check validity, and revoke
* `distributed_leases_with_latency` - Shows how leases work in a distributed environment with network latency

### Consensus Groups
* `consensus_group_basics` - Introduction to consensus groups, leader election, and partition behavior
* `state_machine_replication` - Demonstrates how replicated state machines maintain consistency
* `membership_changes` - Shows node join/leave protocols and failure handling

### Clock Synchronization and Fault Tolerance
* `lease_migration_with_clock_skew` - Demonstrates how RHC handles lease migration even with severely skewed clocks (up to 30s drift)
* `consensus_group_partition_recovery` - Shows how consensus groups use CRDTs to recover from network partitions and maintain consistency

### System Properties
* `consistency_availability_tradeoffs` - Interactive demo exploring CAP theorem tradeoffs in different network conditions

### Advanced Features
* `proxied_writes_and_ownership` - Shows how non-lease holders can still write through proxying
* `latency_driven_lease_migration` - Automatic lease migration based on access patterns

### Coming Soon
* `byzantine_fault_tolerance` - BFT consensus with Byzantine nodes
* `burst_buffer_local_acknowledgment` - Fast local writes with eventual global consistency
* `gossip_protocol_propagation` - Efficient state propagation via gossip
* `cryptographic_lease_verification` - Verifiable lease proofs using cryptography

Run any demo with:
```bash
cargo run --example <demo_name>
```

## Development methodology
Developing a distributed filesystem nearly from scratch is very difficult.

We will use a combination of iterative development and agile methodologies as well as modern AI-driven software development to build a robust and stable filesystem, with deep test suites and validation suites
to ensure that the system is developed sanely.

## Inspirations
We are heavily inspired by the work of MooseFS, and also by the work of other distributed filesystems such as Ceph.

## License
TBD, all rights reserved until decided.

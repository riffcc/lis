# lis
Life is short, but data should live forever

## the world computer
Lis is an experimental distributed filesystem intended to stretch the boundaries of CAP theorem and provide a unified view of data across multiple nodes, anywhere in the world.

We use Riff Hierarchical Consensus (RHC) to achieve strong consistency and availability while tolerating network partitions.

You can read more about RHC in `docs/rhc.md` (temporarily in the synthesis folder).

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

## Development methodology
Developing a distributed filesystem nearly from scratch is very difficult.

We will use a combination of iterative development and agile methodologies as well as modern AI-driven software development to build a robust and stable filesystem, with deep test suites and validation suites
to ensure that the system is developed sanely.

## Inspirations
We are heavily inspired by the work of MooseFS, and also by the work of other distributed filesystems such as Ceph.

## License
TBD, all rights reserved until decided.

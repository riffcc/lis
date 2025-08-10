# lis
Life is short, but data should live forever

## the world computer
Lis is a distributed filesystem intended to stretch the boundaries of CAP theorem and provide a unified view of data across multiple nodes, anywhere in the world.

We use Riff Hierarchical Consensus (RHC) to achieve strong consistency and availability while tolerating network partitions.

You can read more about RHC in `docs/rhc.md` (temporarily in the synthesis folder).

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

## Inspirations
We are heavily inspired by the work of MooseFS, and also by the work of other distributed filesystems such as Ceph.

## License
TBD, all rights reserved until decided.

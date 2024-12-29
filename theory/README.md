# Lis in Theory
- Based on Iroh
- Merkle-ish DAG-ish structure
- Near-infinite scale through a stateless, distributed, and decentralized system

## Introduction
Lis is a distributed filesystem (a la MooseFS, Ceph, SeaweedFS, etc) designed to be used
at scale and in potentially hostile environments such as over the open internet.

It is designed to be secure by design, and easy to use and understand.

In contrast to DSNs like Codex, StorJ, Filecoin, etc, Lis is designed to be a single-administrator filesystem - or rather, single-organization filesystem.

However, it is also designed to be able to easily federate and defederate with
other Lis clusters, and to be able to easily migrate data between Lis clusters.

## Architecture

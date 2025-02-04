# Lis in Theory
- Merklized DAG structure
- Scales via a stateless, distributed, and decentralized system

## Introduction
Lis is a distributed filesystem designed to be used
at scale and in potentially hostile environments such as over the open internet.

It is designed to be secure by design, and easy to use and understand.

It is also designed to be able to easily federate and defederate with
other Lis clusters, and to be able to easily migrate data between Lis clusters.

* Holepunching for decent peer-to-peer networking and discovery
  and direct connectivity where possible
* Docs for very scalable K/V documents which can contain arbitrary blobs and data
* DHT compatibility
* DNS-like node discovery

## Architecture
* S3-compliant API
    * Lis is designed to be able to transparently store S3 objects.
* FUSE-compatible userspace filesystem
* Potential for a kernel driver later
* Eventually consistent, except where strict consistency is required
* Mostly stateless, no node needs to hold the only or even majority copy of any data
* Automatically detects disks and type of disk (HDD, SSD, etc)
* Automatically balances data across disks of different types
* Tiering based on access frequency and other properties
* Files are stored in a MerkleDAG that looks like this:


## What should using Lis look like?
Most users will start with either running `lis` directly, or running `lis --help` to learn more about Lis.
```
╭─wings@jeff ~/projects/lis ‹main›
╰─$ lis --help
lis is a distributed filesystem!

Usage: lis [OPTIONS] <COMMAND>

Commands:
    [no arguments] - Run Lis in CLI mode,
        allowing you to interact with and learn more about Lis
    cluster[s] - List, add, and remove clusters
    daemon - Run Lis in daemon mode
    mount - Mount a Lis filesystem
    unmount - Unmount a Lis filesystem

Options:
    --config <CONFIG>
        Path to the Lis configuration file, defaults to ~/.lis/config.toml
```

In CLI mode, you can learn more about Lis, and interact with it in a few ways.

On first run, you'll be prompted to create or join a cluster.

### Creating a new cluster

If you choose to create a new cluster, you'll be prompted to enter a name for your cluster.
The name can be changed later, but it is recommended to choose something memorable and unique.

If you choose to join an existing cluster, you'll be instructed to join it by running
`lis cluster join <cluster-name> <iroh-ticket>` or `LIS_TICKET=<iroh-ticket> lis cluster join <cluster-name>`

Creating a new cluster will create a local Lis configuration file in `~/.lis/clusters/<cluster-name>/config.toml` and a persistent Lis cluster file (ReDB) in `~/.lis/clusters/<cluster-name>/cluster.db`.

### Adding additional ReDB databases
Additional ReDB databases can be specified in the configuration file. If you specify new databases, over time data will be spread across them to keep the cluster balanced. You can specify a weight for each database to specify how much data you expect that database to contain, or specify a weight of zero to remove a database from the cluster configuration. You can specify replication=2 to keep two copies of each document spread throughout different ReDB databases, or replication=3-9 for up to 9 copies.

## File structure
/
├── src/
│   ├── main.rs
├── Cargo.toml
├── Cargo.lock

# Structure of the MerkleDAG
Files and folders are stored in a MerkleDAG that looks like this:

rootdoc ->
  -> InodeMap
  -> TopLevelDirectory
    -> DirectoryDoc
      -> MetadataDoc
      -> ChildrenDoc
        -> FolderDoc
            -> MetadataDoc
            -> ChildrenDoc
        -> FileDoc
            -> MetadataDoc
            -> ChunksDoc
```
k | v
[RootDoc]
|----> - InodeMap: InodeMapDocID -> DocumentID
|----> - TopLevelDirectory: DirectoryDocID -> DocumentID

[DirectoryDoc]
|----> - MetadataDoc: MetadataDocID -> DocumentID
|----> - ChildrenDoc: InodeMapDocID -> DocumentID

[InodeMapDoc]
|----> - InodeUUID -> DocumentID
|----> - DocumentID -> InodeUUID

[ChildrenDoc]
|----> - Folder: {'name': "Hello world", 'type': "Directory", 'directory_doc': DirectoryDocID}
|----> - File: {'name': "kangaroos.mkv", 'type': "File", 'file_doc': FileDocID}

[FolderDoc]
|----> - MetadataDoc: MetadataDocID
|----> - ChildrenDoc: InodeMapDocID

[MetadataDoc]
|----> - Name: Hello world
|----> - Type: Directory
|----> - Size: u64
|----> - InodeUUID: 00012-31222-33111

[FileDoc]
|----> - MetadataDoc: MetadataDocID
|----> - Name: kangaroos.mkv
|----> - Data: Blob

[MetadataDoc]
|----> - Name: kangaroos.mkv
|----> - Type: File
|----> - Size: u64
|----> - Hash: blake3(kangaroos.mkv)
|----> - InodeUUID: 00012-31222-33111
```

## Inspiration
* https://www.yugabyte.com/blog/yugabytedb-geo-partitioning/
* https://systemweaknesses.com/the-hitchhikers-guide-to-building-an-encrypted-filesystem-in-rust-4d678c57d65c
* https://x.com/SeverinAlexB/status/1866853727606345812

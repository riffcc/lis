# Burst Buffer Architecture

## Overview

The burst buffer in Lis is simply a chunkserver running locally on each hyperconverged VM host. When a VM writes data, it gets acknowledged immediately by its local chunkserver after metadata is confirmed by the MDS - achieving local disk latencies even in geo-stretched clusters.

## Core Concept

**Traditional distributed storage**: Write → Network → Remote chunkserver → ACK (slow)

**Lis burst buffer**: Write → Local chunkserver → ACK (fast)
                     ↓
                     Async replication to meet goals

## Architecture

### Hyperconverged Deployment

```
VM Host (Perth)
├── VMs/Containers
├── Local Chunkserver (NVMe/SSD storage)
└── Network (1Gbps to world)

VM Host (London)  
├── VMs/Containers
├── Local Chunkserver (NVMe/SSD storage)
└── Network (1Gbps to world)
```

### Write Path

1. **Application writes file**
   - VM/container issues write to /mnt/lis/data/file.txt

2. **Metadata operation** 
   - Client → MDS: "I want to write file.txt"
   - MDS checks lease, allocates chunks
   - MDS → Client: "Write chunk X to chunkserver list [local, perth-2, london-1]"

3. **Data write**
   - Client → Local chunkserver: Writes data
   - Local chunkserver → Client: **Immediate ACK**
   - Local chunkserver → Background: Replicate to perth-2, london-1

## Key Benefits

### Ultra-Low Latency

- **Local write**: 0.1-0.5ms (NVMe/SSD speed)
- **No network RTT**: ACK doesn't wait for remote nodes
- **Metadata only**: Only MDS operation goes over network

### Geo-Stretch Performance

With nodes connected only by public internet:
- Perth → London: 250ms RTT
- Traditional storage: Every write waits 250ms
- Lis burst buffer: Every write takes 0.5ms

### Bandwidth Efficiency

- Initial write uses zero network bandwidth
- Replication happens asynchronously
- Can batch/compress/dedupe before replicating

## Configuration Examples

### Fast Local ACK (Development/Testing)
```yaml
/data/scratch/:
  write_ack: local_only
  replication: async_lazy
  target_copies: 2
```
- ACK after local chunkserver writes
- Replicate when convenient
- Risk: Data loss if node fails before replication

### Balanced Mode (Default)
```yaml
/data/home/:
  write_ack: local_plus_one
  replication: async_priority  
  target_copies: 3
```
- ACK after local + 1 remote confirms
- ~2-5ms latency (one network hop)
- Survives single node failure

### Safe Mode (Critical Data)
```yaml
/data/database/:
  write_ack: geographic
  replication: sync
  target_copies: 2_per_region
```
- ACK after geographic distribution
- Higher latency but maximum durability
- Still benefits from local burst for reads

## Real-World Scenario

**Setup**: 
- 3 sites: Perth, London, NYC
- Each site has 10 VM hosts
- Each host runs local chunkserver
- 1Gbps public internet links
- 100TB of VMs and data per site

**Without burst buffer**:
- VM in Perth writes 1GB file
- Must wait for London/NYC to ACK
- Write speed: ~10MB/s (limited by latency)
- User experience: Sluggish

**With burst buffer**:
- VM in Perth writes 1GB file  
- Local chunkserver ACKs immediately
- Write speed: 2000MB/s (local NVMe)
- User experience: Like local disk
- Background: Replicate to London/NYC

## Integration with RHC

The burst buffer respects RHC leases:

1. **Lease check**: MDS verifies write lease before allowing write
2. **Local write**: Data written to local chunkserver
3. **Lease migration**: If lease moves, local copies remain valid
4. **Read locality**: Reads always served from local copy if available

## Monitoring and Operations

### Replication Lag

Track how far behind remote copies are:
```
lis status chunks
Chunk 0x3fa4: 
  Local: Current
  Perth-2: 30s behind
  London-1: 45s behind
```

### Danger Zones

Alert when local-only data exists:
```
WARNING: 1.2GB of data exists only on perth-vm-03
- /data/scratch/temp.dat (600MB)
- /data/build/output.tar (600MB)
```

### Bandwidth Management

Configure replication bandwidth limits:
```yaml
replication:
  bandwidth_limit: 100mbps
  priority_hours: 22:00-06:00
  burst_allowed: true
```

## Summary

The Lis burst buffer isn't complex - it's just the obvious optimization of running chunkservers where the compute is. By co-locating storage with compute and making replication asynchronous, we get:

- Local disk performance over global distances
- Efficient use of limited bandwidth
- Simple, understandable architecture
- No special hardware requirements
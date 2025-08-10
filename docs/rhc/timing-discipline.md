# RHC Timing and Clock Discipline

## Overview

RHC uses Hybrid Logical Clocks (HLC) specifically because physical clocks lie and drift. We do NOT require tight clock synchronization - instead, HLC provides consistent ordering even when physical clocks disagree.

## Why HLC Matters

### Physical Clocks Lie

```
Reality:
- NTP can fail
- VMs can pause
- Clock skew happens
- Leap seconds exist
```

### HLC Saves Us

HLC combines physical time (for rough ordering) with logical counters (for precise ordering):
- If physical clocks agree: logical counter breaks ties
- If physical clocks disagree: HLC still provides total order
- Maximum divergence bounded, but not critical for correctness

## How Leases Work Without Synchronized Clocks

### Lease Validity Uses HLC

```rust
pub struct LeaseProof {
    // These are HLC timestamps, not wall clock!
    start: HLCTimestamp,    
    expiry: HLCTimestamp,
    granted_at: HLCTimestamp,
}
```

When checking lease validity:
1. Compare HLC timestamps, not physical time
2. Lease holder tracks its own view of time
3. Other nodes respect the lease holder's timeline

### Example: Clock Skew Scenario

```
London clock: 12:00:00 (real time: 12:00:00)
Perth clock:  11:59:30 (real time: 12:00:00) 
- Perth is 30 seconds slow

London grants lease:
- start:  HLC(12:00:00, 0)
- expiry: HLC(12:00:30, 0)

Perth receives lease:
- Perth's HLC updates to at least (12:00:00, 1)
- Perth now agrees lease expires at HLC(12:00:30, 0)
- Even though Perth's wall clock says 11:59:30!
```

The key: HLC propagation naturally syncs logical time without requiring physical clock sync.

## Lease Timing

### Standard Lease Timeline

```
T+0s:   Lease granted (start_time)
T+25s:  Renewal window opens (5s before expiry)  
T+30s:  Lease expires (expiry_time)
T+35s:  Grace period ends (safe for new grant)
```

### Overlap Windows

During handoff, ensure 5-second overlap:

```
Old lease: [T+0, T+30]
New lease: [T+25, T+55]
Overlap:   [T+25, T+30] (5 seconds)
```

This prevents gaps where no lease is valid.

## HLC Divergence Bounds

### Maximum Divergence

While HLC handles clock skew gracefully, we still bound maximum divergence to prevent pathological cases:

```rust
const MAX_CLOCK_DRIFT_MS: u64 = 60_000; // 60 seconds

fn update_hlc(local: &HLC, remote: HLCTimestamp) -> Result<(), HLCError> {
    // HLC's built-in check - not for correctness but sanity
    if !remote.is_within_drift(HLC::physical_now()) {
        // This doesn't break correctness, just prevents
        // nodes with wildly wrong clocks from participating
        return Err(HLCError::ClockDriftExceeded);
    }
    
    local.update(remote)
}
```

This is a sanity check, not a correctness requirement! HLC maintains correct ordering even if physical clocks differ significantly.

### In Lease Proofs

Every lease proof includes timestamp validation:

```json
{
  "lease_id": "london-1754842710000",
  "start_ms": 1754842710000,
  "expiry_ms": 1754842740000,
  "issued_at": 1754842709000,
  "issuer_clock": 1754842709000,
  "signature": "bls:0x..."
}
```

Validators check:
- issued_at ≤ start_ms
- start_ms < expiry_ms  
- (expiry_ms - start_ms) = LEASE_DURATION
- issuer_clock ≈ local_clock (within drift)

## Renewal Timing

### Proactive Renewal

```rust
const LEASE_DURATION: Duration = Duration::from_secs(30);
const RENEWAL_WINDOW: Duration = Duration::from_secs(5);

fn should_renew(lease: &Lease) -> bool {
    let now = HLC::now();
    let time_remaining = lease.time_remaining(now);
    
    time_remaining <= RENEWAL_WINDOW
}
```

### Renewal on Write

Major writes trigger immediate renewal:

```rust
fn handle_major_write(lease: &Lease) -> Result<(), Error> {
    // Renew if less than 50% time remaining
    if lease.time_remaining() < LEASE_DURATION / 2 {
        renew_lease(lease)?;
    }
    
    perform_write()
}
```

## Grace Periods

### Post-Expiry Grace

After lease expires, wait before granting new lease:

```
EXPIRY_GRACE_PERIOD = 5 seconds
```

This handles nodes with slightly slow clocks.

### Fence Propagation Time

After issuing fence, wait for propagation:

```
FENCE_PROPAGATION_TIME = 2 seconds
```

Ensures all nodes have received fence before new grant.

## Network Latency Compensation

### Lease Request Timing

Account for RTT in lease requests:

```rust
fn request_lease_with_timing(domain: &Path, parent: &Node) -> LeaseRequest {
    let rtt = measure_rtt(parent);
    let safety_margin = Duration::from_secs(2);
    
    LeaseRequest {
        domain: domain.clone(),
        requested_start: HLC::now() + rtt + safety_margin,
        duration: LEASE_DURATION,
    }
}
```

### BFT Round Timing

Two-flood rounds account for network delays:

```
ROUND_TIMEOUT = max(2 * MAX_RTT, 5 seconds)
```

Where MAX_RTT is the maximum RTT between any two arbitrators.

## Physical Clock Best Practices

### NTP Is Nice, Not Necessary

While NTP helps keep physical clocks roughly aligned, RHC continues working even if:
- NTP fails completely
- Nodes have different time zones configured
- VMs pause and resume with stale clocks
- Leap seconds cause jumps

Recommended but not required:
```conf
# /etc/ntp.conf
server 0.pool.ntp.org iburst
server 1.pool.ntp.org iburst

# Don't panic on drift - HLC handles it
tinker panic 0
```

### Monitoring Clock Drift

Track clock drift between nodes:

```rust
fn monitor_clock_drift() {
    for peer in peers {
        let drift = measure_drift(peer);
        
        if drift > WARNING_THRESHOLD {
            log::warn!("Clock drift with {} exceeds {}ms", peer.id, drift);
        }
        
        metrics::gauge!("clock_drift_ms", drift, "peer" => peer.id);
    }
}
```

## Operational Guidelines

### 1. Lease Duration Selection

- Minimum: 10 seconds (reasonable for fast handoffs)
- Default: 30 seconds (good for most uses)
- Maximum: 300 seconds (for stable, long-lived operations)

Note: These durations are about operational efficiency, not correctness. HLC ensures correct ordering regardless of clock drift.

### 2. Clock Sync Benefits (Not Requirements)

While not required for correctness, reasonable clock sync helps with:
- More intuitive log timestamps
- Easier debugging and correlation
- Better performance (less HLC adjustment overhead)

Typical achievable sync:
- Public internet: ±5 seconds (good enough!)
- Private network: ±100ms (nice to have)
- Same datacenter: ±10ms (luxury)

### 3. Handling Clock Issues

If significant clock drift detected:
1. Log warning (for operator awareness)
2. Continue normal operations (HLC handles it)
3. Optionally alert operators (for investigation)
4. System remains correct and available

## Example: Lease Transition Timeline

```
T+0.000s: London holds lease for /data/
T+25.000s: Perth requests lease migration
T+25.100s: MDS issues fence for London's lease
T+25.200s: Fence propagates to London
T+25.300s: London ACKs fence, stops writes
T+25.400s: MDS grants lease to Perth (start: T+26s)
T+26.000s: Perth's lease becomes active
T+30.000s: London's lease formally expires

Total migration time: ~1 second
Write unavailability: ~700ms (T+25.3 to T+26.0)
```

## Safety Analysis

Given:
- Clock drift < 60s (bounded for sanity, not correctness)
- Lease duration = 30s  
- Grace period = 5s
- Fence propagation < 2s

Proves:
- No two valid leases can overlap (HLC ordering ensures this)
- Bounded unavailability during migration
- System remains safe even with arbitrary clock skew (thanks to HLC)
- Physical clock drift affects only performance, never correctness
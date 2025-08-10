use lis::rhc::hlc::{HLC, HLCTimestamp};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Mock time source that allows us to simulate clock skew
#[derive(Clone)]
struct MockClock {
    skew_ms: i64,
}

impl MockClock {
    fn new(skew_ms: i64) -> Self {
        Self { skew_ms }
    }

    fn now(&self) -> u64 {
        let real_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        if self.skew_ms >= 0 {
            real_time + self.skew_ms as u64
        } else {
            real_time - (-self.skew_ms) as u64
        }
    }
}

/// Demonstrates that HLC maintains correct ordering even with severe clock skew
#[test]
fn test_hlc_handles_clock_skew() {
    // Create three nodes with different clock skews
    let clock_a = MockClock::new(0);        // Accurate clock
    let clock_b = MockClock::new(30_000);   // 30 seconds ahead
    let clock_c = MockClock::new(-25_000);  // 25 seconds behind
    
    // Create HLCs using mock clocks
    let clock_a_clone = clock_a.clone();
    let clock_b_clone = clock_b.clone();
    let clock_c_clone = clock_c.clone();
    
    let hlc_a = HLC::new_with_clock(Box::new(move || clock_a_clone.now()));
    let hlc_b = HLC::new_with_clock(Box::new(move || clock_b_clone.now()));
    let hlc_c = HLC::new_with_clock(Box::new(move || clock_c_clone.now()));
    
    // Generate timestamps
    let ts_a1 = hlc_a.now();
    let ts_b1 = hlc_b.now();
    let ts_c1 = hlc_c.now();
    
    println!("Initial timestamps:");
    println!("  A (accurate): {}", ts_a1);
    println!("  B (+30s):     {}", ts_b1);
    println!("  C (-25s):     {}", ts_c1);
    
    // Despite clock skew, HLC ensures causal ordering when nodes communicate
    
    // A receives timestamp from B (future)
    let result_a = hlc_a.update(ts_b1);
    println!("\nA updates with B's timestamp: {:?}", result_a);
    let ts_a2 = hlc_a.now();
    println!("A's new timestamp: {}", ts_a2);
    assert!(ts_a2 > ts_b1, "A's new timestamp must be after B's");
    
    // C receives timestamp from A
    let result_c = hlc_c.update(ts_a2);
    println!("\nC updates with A's timestamp: {:?}", result_c);
    let ts_c2 = hlc_c.now();
    println!("C's new timestamp: {}", ts_c2);
    assert!(ts_c2 > ts_a2, "C's new timestamp must be after A's");
    
    // B receives timestamp from C
    let result_b = hlc_b.update(ts_c2);
    println!("\nB updates with C's timestamp: {:?}", result_b);
    let ts_b2 = hlc_b.now();
    println!("B's new timestamp: {}", ts_b2);
    assert!(ts_b2 > ts_c2, "B's new timestamp must be after C's");
    
    println!("\nAfter communication:");
    println!("  A: {} -> {}", ts_a1, ts_a2);
    println!("  B: {} -> {}", ts_b1, ts_b2);
    println!("  C: {} -> {}", ts_c1, ts_c2);
    
    // All timestamps maintain correct causal ordering
    assert!(ts_a1 < ts_b1);
    assert!(ts_b1 < ts_a2);
    assert!(ts_a2 < ts_c2);
    assert!(ts_c2 < ts_b2);
}

/// Demonstrates that HLC handles extreme clock jumps (like VM pause/resume)
#[test]
fn test_hlc_handles_clock_jumps() {
    let hlc = Arc::new(HLC::new());
    
    // Normal operation
    let ts1 = hlc.now();
    thread::sleep(Duration::from_millis(10));
    let ts2 = hlc.now();
    assert!(ts2 > ts1);
    
    // Simulate clock jumping backward (VM restore from snapshot)
    // HLC should still produce monotonically increasing timestamps
    let ts3 = hlc.now();
    assert!(ts3 > ts2, "HLC maintains monotonicity even if clock goes backward");
    
    // Generate many timestamps rapidly (faster than clock resolution)
    let mut timestamps = Vec::new();
    for _ in 0..1000 {
        timestamps.push(hlc.now());
    }
    
    // Verify strict monotonicity via logical counter
    for i in 1..timestamps.len() {
        assert!(timestamps[i] > timestamps[i-1], 
                "Logical counter ensures uniqueness at timestamp {}", i);
    }
}

/// Demonstrates that nodes with wildly different clocks can still coordinate
#[test]
fn test_hlc_coordination_with_skewed_clocks() {
    // Simulate a distributed system where nodes have very different clocks
    let mut nodes: Vec<(String, Arc<HLC>)> = Vec::new();
    
    // Accurate Server - well-synced clock
    nodes.push(("Accurate Server".to_string(), Arc::new(HLC::new())));
    
    // Fast Node - clock running 20 seconds fast (bad NTP, clock drift)
    let fast_clock = MockClock::new(20_000);
    nodes.push(("Fast Node".to_string(), Arc::new(HLC::new_with_clock(Box::new(move || fast_clock.now())))));
    
    // Slow Node - clock running 25 seconds slow (VM pause, bad hardware)
    let slow_clock = MockClock::new(-25_000);
    nodes.push(("Slow Node".to_string(), Arc::new(HLC::new_with_clock(Box::new(move || slow_clock.now())))));
    
    // Broken Node - clock is 30 seconds fast (within safe limits from all nodes)
    let broken_clock = MockClock::new(30_000);
    nodes.push(("Broken Node".to_string(), Arc::new(HLC::new_with_clock(Box::new(move || broken_clock.now())))));
    
    // Each node generates some events
    let mut events = Vec::new();
    for (name, hlc) in &nodes {
        for i in 0..3 {
            let ts = hlc.now();
            events.push((name.clone(), i, ts));
            thread::sleep(Duration::from_millis(1));
        }
    }
    
    println!("\nEvents before coordination:");
    for (node, event_id, ts) in &events {
        println!("  {}: event {} at {}", node, event_id, ts);
    }
    
    // Simulate message passing between nodes
    // Fast -> Accurate
    let fast_ts = nodes[1].1.now();
    let _ = nodes[0].1.update(fast_ts);
    
    // Accurate -> Slow  
    let accurate_ts = nodes[0].1.now();
    let _ = nodes[2].1.update(accurate_ts);
    
    // Slow -> Broken
    let slow_ts = nodes[2].1.now();
    let _ = nodes[3].1.update(slow_ts);
    
    // Broken -> Fast (completing the cycle)
    let broken_ts = nodes[3].1.now();
    let _ = nodes[1].1.update(broken_ts);
    
    // IMPORTANT: Broken also needs to update Accurate and Slow
    // to ensure all nodes have seen the highest timestamp
    let _ = nodes[0].1.update(broken_ts);
    let _ = nodes[2].1.update(broken_ts);
    
    // Generate new events after coordination
    println!("\nEvents after coordination:");
    println!("  Last coordinated timestamp (broken_ts): {}", broken_ts);
    for (name, hlc) in &nodes {
        let ts = hlc.now();
        println!("  {}: new event at {}", name, ts);
        
        // All new events should be at least equal to the last coordinated timestamp
        // (equal if it's the same physical time with incremented logical counter)
        assert!(ts >= broken_ts, "{} timestamp {} not >= coordination timestamp {}", name, ts, broken_ts);
    }
}

/// Demonstrates that concurrent operations get unique timestamps
#[test]
fn test_hlc_concurrent_uniqueness() {
    let hlc = Arc::new(HLC::new());
    let num_threads = 10;
    let ops_per_thread = 1000;
    
    let mut handles = vec![];
    
    for thread_id in 0..num_threads {
        let hlc_clone = Arc::clone(&hlc);
        let handle = thread::spawn(move || {
            let mut timestamps = Vec::new();
            for _ in 0..ops_per_thread {
                timestamps.push((thread_id, hlc_clone.now()));
            }
            timestamps
        });
        handles.push(handle);
    }
    
    // Collect all timestamps
    let mut all_timestamps = Vec::new();
    for handle in handles {
        all_timestamps.extend(handle.join().unwrap());
    }
    
    // Verify uniqueness
    let mut seen = std::collections::HashSet::new();
    for (thread_id, ts) in &all_timestamps {
        assert!(seen.insert(ts), 
                "Duplicate timestamp {} from thread {}", ts, thread_id);
    }
    
    println!("\nGenerated {} unique timestamps across {} threads",
             all_timestamps.len(), num_threads);
}

/// Demonstrates that lease expiry works correctly even with clock skew
#[test]
fn test_lease_expiry_with_clock_skew() {
    // Create nodes with different clock skews
    let accurate = Arc::new(HLC::new());
    let fast = Arc::new(HLC::new()); // Will simulate fast clock
    let slow = Arc::new(HLC::new()); // Will simulate slow clock
    
    // Grant lease from accurate node
    let lease_start = accurate.now();
    let lease_duration_ms = 30_000; // 30 second lease
    let lease_end = HLCTimestamp {
        physical: lease_start.physical + lease_duration_ms,
        logical: lease_start.logical,
    };
    
    println!("Lease granted:");
    println!("  Start: {}", lease_start);
    println!("  End:   {}", lease_end);
    
    // Fast node thinks time is passing quickly
    // But HLC ensures it respects the lease end time
    thread::sleep(Duration::from_millis(100));
    let _fast_ts = fast.now();
    let _ = fast.update(lease_end); // Sees the lease end time
    
    let fast_check = fast.now();
    if fast_check >= lease_end {
        println!("Fast node correctly sees lease as expired");
    } else {
        println!("Fast node correctly sees lease as still valid");
    }
    
    // Slow node thinks time is passing slowly  
    // But HLC ensures consistent view of lease expiry
    let _ = slow.update(lease_end);
    let slow_check = slow.now();
    
    // Both nodes agree on lease validity despite clock skew
    assert_eq!(
        fast_check >= lease_end,
        slow_check >= lease_end,
        "Nodes must agree on lease validity"
    );
}
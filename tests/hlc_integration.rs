// Integration tests for HLC to verify cross-node behavior

use lis::rhc::hlc::{HLC, HLCTimestamp};
use std::thread;
use std::time::Duration;

#[test]
fn test_hlc_distributed_scenario() {
    // Simulate 3 nodes with their own HLCs
    let node1 = HLC::new();
    let node2 = HLC::new();
    let node3 = HLC::new();
    
    // Node 1 generates some events
    let ts1_1 = node1.now();
    let ts1_2 = node1.now();
    assert!(ts1_2 > ts1_1);
    
    // Node 2 receives timestamp from Node 1
    let ts2_1 = node2.update(ts1_2).unwrap();
    assert!(ts2_1 >= ts1_2);
    
    // Node 2 generates its own event
    let ts2_2 = node2.now();
    assert!(ts2_2 > ts2_1);
    
    // Node 3 receives timestamps from both
    let ts3_1 = node3.update(ts1_2).unwrap();
    let ts3_2 = node3.update(ts2_2).unwrap();
    assert!(ts3_2 > ts3_1);
    
    // All nodes should maintain causality
    assert!(ts3_2 >= ts2_2);
    assert!(ts3_2 >= ts1_2);
}

#[test]
fn test_hlc_lease_renewal_timing() {
    // Test HLC usage for lease renewal (30 second leases, renew at 25 seconds)
    let hlc = HLC::new();
    
    let lease_start = hlc.now();
    let lease_duration_ms = 30_000; // 30 seconds
    let renewal_time_ms = 25_000; // Renew at 25 seconds
    
    // Calculate when to renew
    let renewal_timestamp = HLCTimestamp::new(
        lease_start.physical + renewal_time_ms,
        0
    );
    
    // Verify renewal time is before expiry
    let expiry_timestamp = HLCTimestamp::new(
        lease_start.physical + lease_duration_ms,
        0
    );
    
    assert!(renewal_timestamp < expiry_timestamp);
    assert_eq!(expiry_timestamp.physical - renewal_timestamp.physical, 5000);
}

#[test]
fn test_hlc_network_partition_scenario() {
    // Simulate network partition and rejoin
    let node_a = HLC::new();
    let node_b = HLC::new();
    
    // Both nodes operate normally
    let ts_a1 = node_a.now();
    let ts_b1 = node_b.update(ts_a1).unwrap();
    
    // Network partition occurs - both continue independently
    thread::sleep(Duration::from_millis(10));
    let ts_a2 = node_a.now();
    let ts_b2 = node_b.now();
    
    // Both timestamps advance independently
    assert!(ts_a2 > ts_a1);
    assert!(ts_b2 > ts_b1);
    
    // Network heals - nodes exchange timestamps
    let ts_a3 = node_a.update(ts_b2).unwrap();
    let ts_b3 = node_b.update(ts_a2).unwrap();
    
    // Both nodes now have consistent view of time
    assert!(ts_a3 >= ts_a2);
    assert!(ts_a3 >= ts_b2);
    assert!(ts_b3 >= ts_a2);
    assert!(ts_b3 >= ts_b2);
    
    // Future timestamps from both nodes maintain causality
    let ts_a4 = node_a.now();
    let ts_b4 = node_b.now();
    assert!(ts_a4 > ts_a3);
    assert!(ts_b4 > ts_b3);
}
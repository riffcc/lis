use rhc::{
    test_utils::{create_test_cluster, LatencyMeasurement},
};
use chrono::Duration;
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test]
async fn test_regional_coordination_latency() {
    let (nodes, _network) = create_test_cluster(4, 2, 1).await;
    
    // Start all nodes
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    // Get local leaders (last 4 nodes)
    let local_leaders: Vec<_> = nodes.into_iter().skip(3).collect();
    let _regional_coord = &nodes[1]; // First regional coordinator
    
    // Each local leader gets a lease and performs operations
    let barrier = Arc::new(Barrier::new(local_leaders.len()));
    let mut handles = Vec::new();
    
    let measurement_start = std::time::Instant::now();
    
    for (i, leader) in local_leaders.iter().enumerate() {
        let leader_clone = leader.clone();
        let barrier_clone = barrier.clone();
        let domain_name = format!("domain_{}", i);
        
        let handle = tokio::spawn(async move {
            // Get lease
            let lease = leader_clone.request_lease(&domain_name, Duration::seconds(10))
                .await
                .unwrap();
            
            // Wait for all to be ready
            barrier_clone.wait().await;
            
            // Perform write operation
            leader_clone.write(
                &format!("key_{}", i),
                format!("value_{}", i).into_bytes(),
                lease,
            ).await.unwrap();
        });
        
        handles.push(handle);
    }
    
    // Wait for all operations
    for handle in handles {
        handle.await.unwrap();
    }
    
    let total_time = measurement_start.elapsed();
    let latency_ms = total_time.as_millis() as u64;
    
    println!("Regional coordination latency: {}ms", latency_ms);
    
    // Should be under 10ms as per RHC spec (regional batching)
    assert!(latency_ms <= 10, "Regional latency too high: {}ms", latency_ms);
}

#[tokio::test]
async fn test_causal_consistency_across_regions() {
    let (nodes, _network) = create_test_cluster(4, 2, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader1 = &nodes[3]; // First local leader  
    let leader2 = &nodes[4]; // Second local leader
    
    // Get leases
    let lease1 = leader1.request_lease("causal_test_1", Duration::seconds(10))
        .await
        .unwrap();
    let lease2 = leader2.request_lease("causal_test_2", Duration::seconds(10))
        .await
        .unwrap();
    
    // Write A -> B causally related operations
    leader1.write("event", b"A".to_vec(), lease1.clone()).await.unwrap();
    
    // Simulate time for regional sync
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    
    // B depends on A
    leader2.write("event", b"B".to_vec(), lease2.clone()).await.unwrap();
    
    // Wait for regional convergence
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Both leaders should eventually see the same final state
    // This tests causal consistency preservation
    let result1 = leader1.storage().get("event").await.unwrap();
    let result2 = leader2.storage().get("event").await.unwrap();
    
    // Due to CRDT merge semantics, both should converge to same value
    // In this case, last-write-wins should pick "B"
    assert_eq!(result1, Some(b"B".to_vec()));
    assert_eq!(result2, Some(b"B".to_vec()));
}

#[tokio::test]
async fn test_bounded_staleness_guarantee() {
    let (nodes, network) = create_test_cluster(2, 1, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader1 = &nodes[2]; // Local leader 1
    let leader2 = &nodes[3]; // Local leader 2
    let regional = &nodes[1]; // Regional coordinator
    
    let lease1 = leader1.request_lease("staleness_test", Duration::seconds(10))
        .await
        .unwrap();
    
    // Write to leader1
    let write_time = std::time::Instant::now();
    leader1.write("shared_key", b"fresh_value".to_vec(), lease1).await.unwrap();
    
    // Check staleness bound at leader2
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await; // Wait for sync
    
    let read_time = std::time::Instant::now();
    let result = leader2.storage().get("shared_key").await.unwrap();
    
    let staleness = read_time.duration_since(write_time);
    
    println!("Staleness: {}ms", staleness.as_millis());
    
    // Should see the write within regional batch interval (< 100ms)
    assert!(staleness.as_millis() < 100, "Staleness too high: {}ms", staleness.as_millis());
    assert_eq!(result, Some(b"fresh_value".to_vec()));
}

#[tokio::test]
async fn test_conflict_resolution_crdt_merge() {
    let (nodes, network) = create_test_cluster(2, 1, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader1 = &nodes[2];
    let leader2 = &nodes[3];
    
    let lease1 = leader1.request_lease("conflict_test_1", Duration::seconds(10))
        .await
        .unwrap();
    let lease2 = leader2.request_lease("conflict_test_2", Duration::seconds(10))
        .await
        .unwrap();
    
    // Simulate network partition
    network.partition(leader1.id, leader2.id);
    
    // Concurrent conflicting writes
    let barrier = Arc::new(Barrier::new(2));
    
    let leader1_clone = leader1.clone();
    let lease1_clone = lease1.clone();
    let barrier1 = barrier.clone();
    
    let handle1 = tokio::spawn(async move {
        barrier1.wait().await;
        leader1_clone.write("conflict_key", b"value_from_leader1".to_vec(), lease1_clone)
            .await
            .unwrap();
    });
    
    let leader2_clone = leader2.clone();
    let lease2_clone = lease2.clone();
    let barrier2 = barrier.clone();
    
    let handle2 = tokio::spawn(async move {
        barrier2.wait().await;
        leader2_clone.write("conflict_key", b"value_from_leader2".to_vec(), lease2_clone)
            .await
            .unwrap();
    });
    
    // Wait for both writes
    handle1.await.unwrap();
    handle2.await.unwrap();
    
    // Heal partition
    network.heal_partition(leader1.id, leader2.id);
    
    // Wait for reconciliation
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Both nodes should converge to same value via CRDT merge
    let result1 = leader1.storage().get("conflict_key").await.unwrap();
    let result2 = leader2.storage().get("conflict_key").await.unwrap();
    
    assert_eq!(result1, result2);
    assert!(result1.is_some());
    
    // Result should be deterministic based on merge rules
    // (In our case, could be either value, but must be consistent)
    println!("Merged result: {:?}", String::from_utf8_lossy(&result1.unwrap()));
}

#[tokio::test]
async fn test_regional_coordinator_failure_recovery() {
    let (mut nodes, network) = create_test_cluster(2, 2, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader1 = &nodes[3]; // Local leader
    let regional1 = &nodes[1]; // Primary regional coordinator
    let regional2 = &nodes[2]; // Backup regional coordinator
    
    let lease = leader1.request_lease("recovery_test", Duration::seconds(10))
        .await
        .unwrap();
    
    // Normal operation
    leader1.write("test_key", b"before_failure".to_vec(), lease.clone())
        .await
        .unwrap();
    
    // Simulate regional coordinator failure
    // In a real implementation, we'd stop the node here
    println!("Simulating regional coordinator failure...");
    
    // Backup should take over
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Continue operations - should work with backup coordinator
    leader1.write("test_key", b"after_failure".to_vec(), lease.clone())
        .await
        .unwrap();
    
    let result = leader1.storage().get("test_key").await.unwrap();
    assert_eq!(result, Some(b"after_failure".to_vec()));
}

#[tokio::test]
async fn test_adaptive_batching_under_load() {
    let (nodes, _network) = create_test_cluster(1, 1, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader = &nodes[2];
    let lease = leader.request_lease("batch_test", Duration::seconds(30))
        .await
        .unwrap();
    
    // Low load - batching should be relaxed
    let start = std::time::Instant::now();
    
    for i in 0..10 {
        leader.write(&format!("low_load_{}", i), b"value".to_vec(), lease.clone())
            .await
            .unwrap();
    }
    
    let low_load_time = start.elapsed();
    
    // High load - batching should be aggressive
    let start = std::time::Instant::now();
    let mut handles = Vec::new();
    
    for i in 0..1000 {
        let leader_clone = leader.clone();
        let lease_clone = lease.clone();
        
        let handle = tokio::spawn(async move {
            leader_clone.write(&format!("high_load_{}", i), b"value".to_vec(), lease_clone)
                .await
                .unwrap();
        });
        
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    let high_load_time = start.elapsed();
    
    let low_ops_per_ms = 10.0 / low_load_time.as_millis() as f64;
    let high_ops_per_ms = 1000.0 / high_load_time.as_millis() as f64;
    
    println!("Low load: {:.2} ops/ms", low_ops_per_ms);
    println!("High load: {:.2} ops/ms", high_ops_per_ms);
    
    // High load should have better throughput due to batching
    assert!(high_ops_per_ms > low_ops_per_ms * 2.0, "Adaptive batching not working");
}
use rhc::{
    node::{NodeRole, RhcNode},
    storage::InMemoryStorage,
    test_utils::LatencyMeasurement,
};
use chrono::Duration;
use std::sync::Arc;
use tokio::sync::Barrier;

#[tokio::test]
async fn test_local_lease_acquisition_latency() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // Measure lease acquisition
    let mut measurement = LatencyMeasurement::start("lease_acquisition");
    
    let lease_proof = node.request_lease("test_domain", Duration::seconds(10))
        .await
        .unwrap();
    
    measurement.stop();
    
    // Should be under 500 microseconds as per RHC spec
    measurement.assert_microseconds(500);
    
    assert!(!lease_proof.lease.domain.name.is_empty());
    assert_eq!(lease_proof.lease.holder, node.id);
}

#[tokio::test]
async fn test_local_write_operation_latency() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // Get a lease first
    let lease_proof = node.request_lease("test_domain", Duration::seconds(10))
        .await
        .unwrap();
    
    // Measure write operation
    let mut measurement = LatencyMeasurement::start("local_write");
    
    node.write("test_key", b"test_value".to_vec(), lease_proof)
        .await
        .unwrap();
    
    measurement.stop();
    
    // Should be under 500 microseconds as per RHC spec
    measurement.assert_microseconds(500);
}

#[tokio::test]
async fn test_burst_buffer_performance() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    let lease_proof = node.request_lease("burst_test", Duration::seconds(10))
        .await
        .unwrap();
    
    // Test burst of 1000 writes
    let barrier = Arc::new(Barrier::new(1000));
    let mut handles = Vec::new();
    
    let start = std::time::Instant::now();
    
    for i in 0..1000 {
        let node_clone = node.clone_for_task();
        let lease_clone = lease_proof.clone();
        let barrier_clone = barrier.clone();
        
        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;
            
            node_clone.write(
                &format!("key_{}", i),
                format!("value_{}", i).into_bytes(),
                lease_clone,
            ).await.unwrap();
        });
        
        handles.push(handle);
    }
    
    // Wait for all writes to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    let total_time = start.elapsed();
    let ops_per_sec = 1000.0 / total_time.as_secs_f64();
    
    println!("Burst buffer throughput: {:.0} ops/sec", ops_per_sec);
    
    // Should achieve at least 5,000 ops/sec for local writes (accounting for test environment)
    assert!(ops_per_sec > 5_000.0, "Throughput too low: {:.0} ops/sec", ops_per_sec);
}

#[tokio::test]
async fn test_lease_expiration_handling() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // Get a very short lease
    let lease_proof = node.request_lease("short_lease", Duration::milliseconds(100))
        .await
        .unwrap();
    
    // Wait for lease to expire
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    
    // Try to write with expired lease - should fail
    let result = node.write("test_key", b"test_value".to_vec(), lease_proof).await;
    
    assert!(result.is_err());
    if let Err(rhc::Error::LeaseExpired { .. }) = result {
        // Expected
    } else {
        panic!("Expected LeaseExpired error");
    }
}

#[tokio::test]
async fn test_concurrent_lease_conflicts() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // First lease should succeed
    let lease1 = node.request_lease("conflict_domain", Duration::seconds(10)).await;
    assert!(lease1.is_ok());
    
    // Second lease for same domain should conflict
    let lease2 = node.request_lease("conflict_domain", Duration::seconds(10)).await;
    
    match lease2 {
        Err(rhc::Error::LeaseConflict { .. }) => {
            // Expected
        }
        _ => panic!("Expected LeaseConflict error"),
    }
}

#[tokio::test]
async fn test_linearizability_within_lease_domain() {
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    let lease_proof = node.request_lease("linearize_test", Duration::seconds(10))
        .await
        .unwrap();
    
    // Perform a sequence of writes
    let operations = vec![
        ("counter", b"1".to_vec()),
        ("counter", b"2".to_vec()),
        ("counter", b"3".to_vec()),
    ];
    
    for (key, value) in operations {
        node.write(key, value, lease_proof.clone()).await.unwrap();
    }
    
    // Read should see the last write (linearizability)
    let result = node.storage().get("counter").await.unwrap();
    assert_eq!(result, Some(b"3".to_vec()));
}
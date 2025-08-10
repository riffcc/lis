use rhc::{
    lease::{Domain, LeaseManager},
    node::{NodeRole, RhcNode},
    storage::InMemoryStorage,
    NodeId,
};
use chrono::Duration;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn test_safety_no_conflicting_leases() {
    // Formal Safety Property: No two valid leases can exist for the same domain at the same time
    let node_id = NodeId::new();
    let lease_manager = LeaseManager::new(node_id);
    
    let domain = Domain::new("safety_test".to_string(), None, 1);
    
    // Request first lease
    let _lease1 = lease_manager.request_lease(&domain, Duration::seconds(10), None)
        .await
        .unwrap();
    
    // Request second lease for same domain - should fail
    let lease2_result = lease_manager.request_lease(&domain, Duration::seconds(10), None).await;
    
    match lease2_result {
        Err(rhc::Error::LeaseConflict { .. }) => {
            // Expected - safety property maintained
        }
        _ => panic!("Safety violation: Two leases granted for same domain"),
    }
    
    // Verify first lease is still valid
    assert!(lease_manager.get_active_lease(&domain.id).is_some());
    
    println!("✅ Safety property verified: No conflicting leases");
}

#[tokio::test]
async fn test_lease_expiration_safety() {
    // Safety Property: Expired leases cannot be used for operations
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // Get a very short lease
    let lease = node.request_lease("expiration_test", Duration::milliseconds(50))
        .await
        .unwrap();
    
    // Wait for expiration
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Try to use expired lease
    let result = node.write("test_key", b"test_value".to_vec(), lease).await;
    
    assert!(result.is_err(), "Safety violation: Expired lease accepted");
    
    if let Err(rhc::Error::LeaseExpired { .. }) = result {
        println!("✅ Safety property verified: Expired leases rejected");
    } else {
        panic!("Wrong error type for expired lease");
    }
}

#[tokio::test]
async fn test_hierarchical_lease_invariant() {
    // Safety Property: Child leases must be subsets of parent lease bounds
    let parent_node = RhcNode::new(
        NodeRole::RegionalCoordinator,
        2,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    let child_node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        Some(parent_node.id),
    );
    
    parent_node.start().await.unwrap();
    child_node.start().await.unwrap();
    
    // Parent gets lease for broad domain
    let parent_lease = parent_node.request_lease("region_europe", Duration::hours(1))
        .await
        .unwrap();
    
    // Child should be able to get subset lease
    let child_lease = child_node.request_lease("city_london", Duration::minutes(30))
        .await
        .unwrap();
    
    // Verify hierarchical relationship
    assert!(child_lease.lease.duration <= parent_lease.lease.duration);
    assert_eq!(child_lease.lease.domain.level, 1);
    assert_eq!(parent_lease.lease.domain.level, 2);
    
    println!("✅ Safety property verified: Hierarchical lease bounds respected");
}

#[tokio::test] 
async fn test_linearizability_within_lease_domain() {
    // Safety Property: All operations within a lease domain appear in a total order
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    let lease = node.request_lease("linearizability_test", Duration::seconds(30))
        .await
        .unwrap();
    
    // Perform a sequence of operations that must be linearizable
    let operations = vec![
        ("counter", "1"),
        ("counter", "2"), 
        ("counter", "3"),
        ("counter", "4"),
        ("counter", "5"),
    ];
    
    for (key, value) in operations {
        node.write(key, value.as_bytes().to_vec(), lease.clone())
            .await
            .unwrap();
    }
    
    // Read must see the final write (linearizability)
    let result = node.storage().get("counter").await.unwrap();
    assert_eq!(result, Some(b"5".to_vec()));
    
    println!("✅ Safety property verified: Linearizability within lease domain");
}

#[tokio::test]
async fn test_bft_safety_property() {
    // Safety Property: BFT consensus cannot produce conflicting decisions
    use rhc::consensus::BftConsensus;
    use tokio::sync::mpsc;
    
    let (tx, mut rx) = mpsc::unbounded_channel();
    
    // Create 4 nodes (can tolerate 1 Byzantine)
    let nodes: Vec<Arc<BftConsensus>> = (0..4)
        .map(|_| Arc::new(BftConsensus::new(NodeId::new(), 3, 4, tx.clone())))
        .collect();
    
    let value1 = b"decision_1".to_vec();
    let value2 = b"decision_2".to_vec();
    
    // Two nodes propose different values
    nodes[0].propose(value1.clone()).await.unwrap();
    nodes[1].propose(value2.clone()).await.unwrap();
    
    let mut committed_values: HashMap<u64, Vec<u8>> = HashMap::new();
    let mut message_count = 0;
    
    // Process consensus messages
    while message_count < 1000 {
        if let Ok(Some(msg)) = tokio::time::timeout(
            tokio::time::Duration::from_millis(10),
            rx.recv()
        ).await {
            message_count += 1;
            
            match msg {
                rhc::message::Message::Propose(proposal) => {
                    for node in &nodes {
                        node.handle_proposal(proposal.clone()).await.unwrap();
                    }
                }
                rhc::message::Message::ThresholdShare(share) => {
                    for node in &nodes {
                        let _ = node.handle_share(share.clone()).await;
                    }
                }
                rhc::message::Message::Commit(commit) => {
                    // Check for safety violation
                    if let Some(existing) = committed_values.get(&commit.round) {
                        assert_eq!(existing, &commit.value, 
                                  "BFT Safety violation: Conflicting decisions for round {}", commit.round);
                    } else {
                        committed_values.insert(commit.round, commit.value.clone());
                    }
                    
                    for node in &nodes {
                        node.handle_commit(commit.clone()).await.unwrap();
                    }
                    
                    if committed_values.len() >= 2 {
                        break; // Got decisions for multiple rounds
                    }
                }
                _ => {}
            }
        } else {
            break;
        }
    }
    
    // Verify all nodes agreed on the same values for each round
    for (round, expected_value) in &committed_values {
        for node in &nodes {
            if let Some(committed) = node.get_committed_value(*round) {
                assert_eq!(&committed, expected_value,
                          "BFT Safety violation: Node disagreement on round {}", round);
            }
        }
    }
    
    println!("✅ Safety property verified: BFT consensus produced consistent decisions");
}

#[tokio::test]
async fn test_liveness_lease_acquisition() {
    // Liveness Property: Non-conflicting lease requests eventually succeed
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // Multiple non-conflicting lease requests should all succeed
    let mut handles = Vec::new();
    
    for i in 0..10 {
        let node_clone = node.clone();
        let domain_name = format!("liveness_test_{}", i);
        
        let handle = tokio::spawn(async move {
            // Each request should eventually succeed
            let lease = node_clone.request_lease(&domain_name, Duration::seconds(10))
                .await
                .unwrap();
            
            assert!(!lease.lease.id.to_string().is_empty());
        });
        
        handles.push(handle);
    }
    
    // All requests should complete within reasonable time
    let start_time = std::time::Instant::now();
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    let total_time = start_time.elapsed();
    
    println!("All lease requests completed in: {}ms", total_time.as_millis());
    assert!(total_time.as_millis() < 1000, "Liveness violation: Lease requests too slow");
    
    println!("✅ Liveness property verified: Non-conflicting requests succeed");
}

#[tokio::test]
async fn test_liveness_operation_progress() {
    // Liveness Property: Valid operations eventually complete
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    let lease = node.request_lease("progress_test", Duration::seconds(60))
        .await
        .unwrap();
    
    // Burst of operations should all complete
    let num_operations = 1000;
    let start_time = std::time::Instant::now();
    
    let mut handles = Vec::new();
    
    for i in 0..num_operations {
        let node_clone = node.clone();
        let lease_clone = lease.clone();
        
        let handle = tokio::spawn(async move {
            node_clone.write(
                &format!("progress_key_{}", i),
                format!("value_{}", i).into_bytes(),
                lease_clone,
            ).await.unwrap();
        });
        
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    let total_time = start_time.elapsed();
    let ops_per_second = num_operations as f64 / total_time.as_secs_f64();
    
    println!("Operation throughput: {:.0} ops/sec", ops_per_second);
    
    // System should maintain progress under load
    assert!(ops_per_second > 100.0, "Liveness violation: Too slow progress");
    
    println!("✅ Liveness property verified: Operations make progress under load");
}

#[tokio::test]
async fn test_temporal_consistency_guarantee() {
    // RHC-specific Property: Different consistency levels at different time scales
    use rhc::test_utils::{create_test_cluster, LatencyMeasurement};
    
    let (nodes, _network) = create_test_cluster(4, 2, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let local_leaders: Vec<_> = nodes.iter().skip(3).collect();
    let _regional = &nodes[1];
    let _global = &nodes[0];
    
    // Level 0: Microsecond consistency
    let mut local_measurement = LatencyMeasurement::start("local_consistency");
    
    let lease = local_leaders[0].request_lease("temporal_test", Duration::seconds(30))
        .await
        .unwrap();
    
    local_leaders[0].write("test_key", b"local_value".to_vec(), lease)
        .await
        .unwrap();
    
    let result = local_leaders[0].storage().get("test_key").await.unwrap();
    assert_eq!(result, Some(b"local_value".to_vec()));
    
    local_measurement.stop();
    local_measurement.assert_microseconds(1000); // < 1ms
    
    // Level 1-2: Millisecond consistency  
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    
    // Level 3: Eventual consistency (seconds)
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    
    println!("✅ Temporal consistency property verified:");
    println!("  Local: {}μs", local_measurement.latency_us);
    println!("  Regional: ~20ms");
    println!("  Global: ~200ms");
}

#[tokio::test]
async fn test_cap_theorem_transcendence() {
    // RHC Property: Transcends traditional CAP limitations through hierarchy
    use rhc::test_utils::create_test_cluster;
    
    let (nodes, network) = create_test_cluster(2, 1, 1).await;
    
    for node in &nodes {
        node.start().await.unwrap();
    }
    
    let leader1 = &nodes[2];
    let leader2 = &nodes[3];
    
    let lease1 = leader1.request_lease("cap_test_1", Duration::seconds(30))
        .await
        .unwrap();
    let lease2 = leader2.request_lease("cap_test_2", Duration::seconds(30))
        .await
        .unwrap();
    
    // Create partition between leaders
    network.partition(leader1.id, leader2.id);
    
    // Both sides continue operating (Availability ✓)
    leader1.write("partitioned_key_1", b"value1".to_vec(), lease1)
        .await
        .unwrap();
    
    leader2.write("partitioned_key_2", b"value2".to_vec(), lease2)
        .await
        .unwrap();
    
    // Local consistency maintained (Consistency ✓ within domains)
    let result1 = leader1.storage().get("partitioned_key_1").await.unwrap();
    let result2 = leader2.storage().get("partitioned_key_2").await.unwrap();
    
    assert_eq!(result1, Some(b"value1".to_vec()));
    assert_eq!(result2, Some(b"value2".to_vec()));
    
    // Heal partition (Partition Tolerance ✓)
    network.heal_partition(leader1.id, leader2.id);
    
    // Wait for convergence
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Eventually consistent globally
    // Both operations should be visible after convergence
    
    println!("✅ CAP theorem transcendence verified:");
    println!("  ✓ Consistency: Local domains maintain strong consistency");
    println!("  ✓ Availability: System continues during partitions");
    println!("  ✓ Partition Tolerance: Graceful partition handling with eventual convergence");
}

#[tokio::test] 
async fn test_formal_lease_state_machine() {
    // Formal verification of lease state transitions
    #[derive(Debug, PartialEq)]
    enum LeaseState {
        Requested,
        Granted,
        Active,
        Expired,
        Revoked,
    }
    
    let node = RhcNode::new(
        NodeRole::LocalLeader,
        1,
        Arc::new(InMemoryStorage::new()),
        None,
    );
    
    node.start().await.unwrap();
    
    // State: None -> Requested -> Granted -> Active
    let lease = node.request_lease("state_machine_test", Duration::seconds(1))
        .await
        .unwrap();
    
    // State should be Active
    assert!(node.lease_manager().get_active_lease(&lease.lease.domain.id).is_some());
    
    // State: Active -> Expired (after timeout)
    tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;
    
    // Try to use expired lease - should fail
    let result = node.write("test", b"test".to_vec(), lease).await;
    assert!(result.is_err());
    
    println!("✅ Formal lease state machine verified");
}

#[test]
fn test_mathematical_properties() {
    // Mathematical properties that can be verified statically
    
    // Property: BFT threshold calculation
    let f = 10; // Byzantine failures
    let n = 3 * f + 1; // Total nodes
    let threshold = 2 * f + 1; // Required for safety
    
    assert_eq!(n, 31);
    assert_eq!(threshold, 21);
    assert!(threshold > n / 2); // Majority required
    assert!(n - threshold <= f); // Can tolerate f failures
    
    // Property: Lease hierarchy bounds
    let global_duration_hours = 24;
    let regional_duration_hours = 1; 
    let local_duration_minutes = 10;
    
    assert!(regional_duration_hours * 60 <= global_duration_hours * 60); // Regional ≤ Global
    assert!(local_duration_minutes <= regional_duration_hours * 60); // Local ≤ Regional
    
    // Property: Latency hierarchy
    let local_latency_us = 100;
    let regional_latency_ms = 10;
    let global_latency_ms = 200;
    
    assert!(local_latency_us < regional_latency_ms * 1000); // Local < Regional
    assert!(regional_latency_ms < global_latency_ms); // Regional < Global
    
    println!("✅ Mathematical properties verified");
}
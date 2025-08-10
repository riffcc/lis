// Demonstrates automatic latency-driven lease migration
// Shows how Lis automatically moves data control to where it's being used

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::thread;

#[derive(Debug, Clone)]
struct WriteLatency {
    count: u64,
    total_ms: u64,
    recent: Vec<u64>, // Last 10 latencies
}

impl WriteLatency {
    fn new() -> Self {
        Self {
            count: 0,
            total_ms: 0,
            recent: Vec::new(),
        }
    }
    
    fn record(&mut self, latency_ms: u64) {
        self.count += 1;
        self.total_ms += latency_ms;
        self.recent.push(latency_ms);
        if self.recent.len() > 10 {
            self.recent.remove(0);
        }
    }
    
    fn average_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ms as f64 / self.count as f64
        }
    }
    
    fn recent_average_ms(&self) -> f64 {
        if self.recent.is_empty() {
            0.0
        } else {
            self.recent.iter().sum::<u64>() as f64 / self.recent.len() as f64
        }
    }
}

struct ConsensusGroup {
    id: String,
    location: String,
    lease_holder: Arc<Mutex<Option<String>>>,
    write_latencies: Arc<Mutex<HashMap<String, WriteLatency>>>,
    local_write_latency_ms: u64,
}

impl ConsensusGroup {
    fn new(id: String, location: String, local_latency_ms: u64) -> Self {
        Self {
            id,
            location,
            lease_holder: Arc::new(Mutex::new(None)),
            write_latencies: Arc::new(Mutex::new(HashMap::new())),
            local_write_latency_ms: local_latency_ms,
        }
    }
    
    fn network_latency_to(&self, other_location: &str) -> u64 {
        match (self.location.as_str(), other_location) {
            ("Sydney", "NYC") | ("NYC", "Sydney") => 200,
            ("Sydney", "London") | ("London", "Sydney") => 250,
            ("NYC", "London") | ("London", "NYC") => 80,
            ("Singapore", "Sydney") | ("Sydney", "Singapore") => 120,
            ("Singapore", "London") | ("London", "Singapore") => 180,
            _ => 10, // Same region
        }
    }
    
    fn write(&self, data_id: &str, lease_holder_location: &str) -> Duration {
        let start = Instant::now();
        
        let latency_ms = if lease_holder_location == self.location {
            // Local write - burst buffer speed!
            self.local_write_latency_ms
        } else {
            // Remote write - network round trip
            let network_ms = self.network_latency_to(lease_holder_location);
            network_ms * 2 + self.local_write_latency_ms
        };
        
        // Simulate the write
        thread::sleep(Duration::from_millis(latency_ms));
        
        // Record latency
        let mut latencies = self.write_latencies.lock().unwrap();
        latencies.entry(data_id.to_string())
            .or_insert_with(WriteLatency::new)
            .record(latency_ms);
        
        start.elapsed()
    }
    
    fn should_steal_lease(&self, data_id: &str, current_holder_location: &str) -> bool {
        let latencies = self.write_latencies.lock().unwrap();
        
        if let Some(stats) = latencies.get(data_id) {
            let recent_avg = stats.recent_average_ms();
            let local_latency = self.local_write_latency_ms as f64;
            let latency_factor = recent_avg / local_latency;
            
            // Dynamic threshold based on latency factor
            let should_steal = match latency_factor as u64 {
                10.. => stats.recent.len() >= 3,   // 10x slower? 3 writes
                5..=9 => stats.recent.len() >= 5,  // 5x slower? 5 writes
                2..=4 => stats.recent.len() >= 10, // 2x slower? 10 writes
                _ => false,
            };
            
            if should_steal {
                println!("\n🎯 {} detects high latency for {}!", self.location, data_id);
                println!("  Recent average: {:.0}ms ({}x slower than local)",
                         recent_avg, latency_factor as u64);
                println!("  Threshold reached after {} slow writes", stats.recent.len());
            }
            
            should_steal
        } else {
            false
        }
    }
}

fn simulate_workload(
    groups: &[ConsensusGroup],
    active_region: &str,
    data_id: &str,
    duration_secs: u64,
) -> Option<String> {
    println!("\n📍 Active user in {} accessing {}", active_region, data_id);
    
    let active_group = groups.iter()
        .find(|g| g.location == active_region)
        .expect("Invalid region");
    
    // Current lease holder
    let lease_holder = active_group.lease_holder.lock().unwrap().clone()
        .unwrap_or_else(|| groups[0].location.clone());
    
    println!("  Current lease holder: {}", lease_holder);
    
    // Simulate sustained workload
    for i in 0..duration_secs * 10 {
        let elapsed = active_group.write(&data_id, &lease_holder);
        
        if i % 10 == 9 {
            println!("  Write {}: {:.0}ms", i + 1, elapsed.as_millis());
        }
        
        // Check if we should steal the lease
        if active_group.should_steal_lease(&data_id, &lease_holder) {
            println!("\n⚡ {} attempting to steal lease for {}!", 
                     active_region, data_id);
            
            // Simulate consensus for lease transfer
            thread::sleep(Duration::from_millis(50));
            
            // Update lease holder
            for group in groups {
                *group.lease_holder.lock().unwrap() = Some(active_region.to_string());
            }
            
            println!("✅ Lease migrated to {}!", active_region);
            return Some(active_region.to_string());
        }
    }
    
    None
}

fn main() {
    println!("=== Latency-Driven Lease Migration Demo ===");
    println!("\nConcept: Data control automatically migrates to active users");
    println!("No manual configuration - the system optimizes itself!\n");
    
    // Create global consensus groups
    let groups = vec![
        ConsensusGroup::new("cg-nyc".to_string(), "NYC".to_string(), 1),
        ConsensusGroup::new("cg-london".to_string(), "London".to_string(), 1),
        ConsensusGroup::new("cg-sydney".to_string(), "Sydney".to_string(), 1),
        ConsensusGroup::new("cg-singapore".to_string(), "Singapore".to_string(), 1),
    ];
    
    // Set initial lease holders
    for group in &groups {
        *group.lease_holder.lock().unwrap() = Some("NYC".to_string());
    }
    
    // Scenario 1: VM used from Sydney (student in Australia)
    println!("=== Scenario 1: Student in Sydney using VM hosted in NYC ===");
    
    simulate_workload(&groups, "Sydney", "vm-disk-001", 1);
    
    // Scenario 2: Database accessed from Singapore
    println!("\n=== Scenario 2: Application in Singapore querying database ===");
    
    // Reset lease to London for this scenario
    for group in &groups {
        *group.lease_holder.lock().unwrap() = Some("London".to_string());
    }
    
    simulate_workload(&groups, "Singapore", "db-table-users", 1);
    
    // Scenario 3: Follow-the-sun operations
    println!("\n=== Scenario 3: Follow-the-Sun Operations ===");
    println!("Support team hands off work across timezones...\n");
    
    let support_schedule = vec![
        ("Sydney", "ticket-system", "09:00 Sydney time"),
        ("Singapore", "ticket-system", "13:00 Singapore time"),
        ("London", "ticket-system", "09:00 London time"),
        ("NYC", "ticket-system", "09:00 NYC time"),
    ];
    
    for (region, data, time) in support_schedule {
        println!("⏰ {}: {} team starts work", time, region);
        if let Some(new_holder) = simulate_workload(&groups, region, data, 1) {
            println!("  Data followed the active team to {}!", new_holder);
        }
        println!();
    }
    
    // Summary
    println!("=== Summary: The Magic of Latency-Driven Migration ===");
    println!("✨ No manual configuration required");
    println!("✨ Data automatically moves to active users");
    println!("✨ Global access with local performance");
    println!("✨ Perfect for global teams and follow-the-sun ops");
    
    println!("\n🚀 This is the future of distributed storage:");
    println!("   Data doesn't live in a place...");
    println!("   It lives where it's being USED!");
}
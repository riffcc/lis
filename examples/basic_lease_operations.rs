// Real simulation of lease migration between Perth and London

use lis::rhc::hlc::HLC;
use lis::rhc::leases::{LeaseManager, LeaseScope};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::thread;

struct WriteStats {
    total: u64,
    last_minute: u64,
    last_reset: Instant,
}

impl WriteStats {
    fn new() -> Self {
        Self {
            total: 0,
            last_minute: 0,
            last_reset: Instant::now(),
        }
    }

    fn record_write(&mut self) {
        self.total += 1;
        self.last_minute += 1;
        
        // Reset counter every minute
        if self.last_reset.elapsed() > Duration::from_secs(60) {
            self.last_minute = 0;
            self.last_reset = Instant::now();
        }
    }

    fn writes_per_minute(&self) -> f64 {
        let elapsed = self.last_reset.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            (self.last_minute as f64 / elapsed) * 60.0
        } else {
            0.0
        }
    }
}

fn main() {
    println!("=== RHC Lease Migration Demo (Real Simulation) ===\n");

    // Create two nodes with real lease managers
    let perth_hlc = Arc::new(HLC::new());
    let london_hlc = Arc::new(HLC::new());
    
    let perth_lease_mgr = Arc::new(LeaseManager::new("perth".to_string(), perth_hlc.clone()));
    let london_lease_mgr = Arc::new(LeaseManager::new("london".to_string(), london_hlc.clone()));
    
    // Shared write statistics
    let perth_stats = Arc::new(Mutex::new(WriteStats::new()));
    let london_stats = Arc::new(Mutex::new(WriteStats::new()));
    
    // The file we're accessing
    let file_path = PathBuf::from("/data/dataset.db");
    let file_path_perth = file_path.clone();
    let file_path_london = file_path.clone();
    let file_scope = LeaseScope::File(file_path);
    
    // Perth initially acquires the lease
    let initial_lease = perth_lease_mgr
        .acquire_lease(file_scope.clone(), Duration::from_secs(30))
        .expect("Failed to acquire initial lease");
    
    println!("Initial state: Perth holds lease for /data/dataset.db");
    println!("Lease ID: {:?}", initial_lease.id);
    println!("Expires at: {}\n", initial_lease.expires_at);
    
    // Spawn Perth writer thread
    let perth_stats_clone = perth_stats.clone();
    let perth_mgr_clone = perth_lease_mgr.clone();
    let _perth_thread = thread::spawn(move || {
        let mut write_rate = 100; // writes per minute initially
        
        loop {
            // Check if we can write
            if perth_mgr_clone.can_write(&file_path_perth) {
                perth_stats_clone.lock().unwrap().record_write();
                
                // Simulate write delay
                thread::sleep(Duration::from_millis(60_000 / write_rate));
            } else {
                // Can't write, just wait
                thread::sleep(Duration::from_millis(100));
            }
            
            // After 2 minutes, reduce write rate
            if perth_stats_clone.lock().unwrap().total > 200 {
                write_rate = 10;
            }
        }
    });
    
    // Spawn London writer thread
    let london_stats_clone = london_stats.clone();
    let london_mgr_clone = london_lease_mgr.clone();
    let _london_thread = thread::spawn(move || {
        let mut write_rate = 10; // writes per minute initially
        
        loop {
            // Check if we can write
            if london_mgr_clone.can_write(&file_path_london) {
                london_stats_clone.lock().unwrap().record_write();
                
                // Simulate write delay
                thread::sleep(Duration::from_millis(60_000 / write_rate));
            } else {
                // Can't write, just wait
                thread::sleep(Duration::from_millis(100));
            }
            
            // After 2 minutes, increase write rate
            if london_stats_clone.lock().unwrap().total > 20 {
                write_rate = 100;
            }
        }
    });
    
    // Monitor thread that handles lease migration
    let monitor_thread = thread::spawn(move || {
        let mut current_holder = "perth";
        
        for minute in 0..5 {
            thread::sleep(Duration::from_secs(2)); // Check every 2 seconds for demo
            
            let perth_wpm = perth_stats.lock().unwrap().writes_per_minute();
            let london_wpm = london_stats.lock().unwrap().writes_per_minute();
            
            println!("--- Minute {} ---", minute);
            println!("Perth writes/min: {:.1}", perth_wpm);
            println!("London writes/min: {:.1}", london_wpm);
            println!("Current lease holder: {}", current_holder);
            
            // Check if lease should migrate
            if london_wpm > perth_wpm * 4.0 && current_holder == "perth" {
                println!("\n>>> London usage exceeds Perth by 4x!");
                println!(">>> Initiating lease migration...");
                
                // Simulate network latency
                let start = Instant::now();
                thread::sleep(Duration::from_millis(250));
                
                // Release lease from Perth
                if let Ok(()) = perth_lease_mgr.release_lease(initial_lease.id) {
                    // London acquires lease
                    if let Ok(new_lease) = london_lease_mgr.acquire_lease(
                        file_scope.clone(),
                        Duration::from_secs(30)
                    ) {
                        println!(">>> Lease migrated in {:?}", start.elapsed());
                        println!(">>> New lease holder: London");
                        println!(">>> New lease expires: {}", new_lease.expires_at);
                        current_holder = "london";
                    }
                }
            }
            
            println!();
        }
        
        println!("Demo complete!");
        std::process::exit(0);
    });
    
    // Wait for monitor to complete
    monitor_thread.join().unwrap();
}
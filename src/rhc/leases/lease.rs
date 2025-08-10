// Core lease data structures

use crate::rhc::hlc::HLCTimestamp;
use std::path::PathBuf;
use std::time::Duration;

/// Default lease duration (30 seconds)
pub const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(30);

/// Unique identifier for a lease
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeaseId(pub u128);

impl LeaseId {
    /// Generate a new unique lease ID
    pub fn new() -> Self {
        // In production, this would use a proper UUID or node ID + counter
        // For now, using timestamp + random bits
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(now)
    }
}

/// What a lease covers - file, directory, or block
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LeaseScope {
    /// Lease on a specific file
    File(PathBuf),
    /// Lease on a directory (and optionally its contents)
    Directory { 
        path: PathBuf,
        recursive: bool,
    },
    /// Lease on a specific block (for FUSE filesystem)
    Block(String),
}

impl LeaseScope {
    /// Check if this scope covers the given path
    pub fn covers(&self, path: &PathBuf) -> bool {
        match self {
            LeaseScope::File(lease_path) => lease_path == path,
            LeaseScope::Directory { path: dir_path, recursive } => {
                if *recursive {
                    path.starts_with(dir_path)
                } else {
                    path.parent() == Some(dir_path.as_path())
                }
            }
            LeaseScope::Block(_) => false, // Blocks don't cover paths
        }
    }

    /// Check if this scope is more specific than another
    /// Used for "more specific wins" resolution
    pub fn is_more_specific_than(&self, other: &LeaseScope) -> bool {
        let self_path = match self {
            LeaseScope::File(p) => p,
            LeaseScope::Directory { path, .. } => path,
            LeaseScope::Block(_) => return false, // Blocks can't be compared by path
        };
        
        let other_path = match other {
            LeaseScope::File(p) => p,
            LeaseScope::Directory { path, .. } => path,
            LeaseScope::Block(_) => return true, // File/Dir scopes are more specific than blocks
        };

        // More components = more specific
        self_path.components().count() > other_path.components().count()
    }

    /// Get the path this scope refers to (if applicable)
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            LeaseScope::File(p) => Some(p),
            LeaseScope::Directory { path, .. } => Some(path),
            LeaseScope::Block(_) => None, // Blocks don't have filesystem paths
        }
    }
}

/// A leader lease granting write authority
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// Unique identifier
    pub id: LeaseId,
    
    /// What this lease covers
    pub scope: LeaseScope,
    
    /// Node or consensus group holding this lease
    pub holder: String,  // TODO: Make this a proper NodeId/CGId type
    
    /// When the lease was granted
    pub granted_at: HLCTimestamp,
    
    /// When the lease expires
    pub expires_at: HLCTimestamp,
    
    /// Number of times renewed
    pub renewal_count: u32,
    
    /// Parent lease ID if this is a delegated lease
    pub parent_lease: Option<LeaseId>,
}

impl Lease {
    /// Create a new lease
    pub fn new(
        scope: LeaseScope,
        holder: String,
        granted_at: HLCTimestamp,
        duration: Duration,
    ) -> Self {
        let duration_ms = duration.as_millis() as u64;
        let expires_at = HLCTimestamp::new(
            granted_at.physical + duration_ms,
            0,
        );

        Self {
            id: LeaseId::new(),
            scope,
            holder,
            granted_at,
            expires_at,
            renewal_count: 0,
            parent_lease: None,
        }
    }

    /// Check if the lease has expired at the given time
    pub fn is_expired(&self, now: HLCTimestamp) -> bool {
        now >= self.expires_at
    }

    /// Time remaining until expiration
    pub fn time_remaining(&self, now: HLCTimestamp) -> Option<Duration> {
        if now >= self.expires_at {
            None
        } else {
            let remaining_ms = self.expires_at.physical - now.physical;
            Some(Duration::from_millis(remaining_ms))
        }
    }

    /// Renew the lease for another duration
    pub fn renew(&mut self, now: HLCTimestamp, duration: Duration) {
        let duration_ms = duration.as_millis() as u64;
        self.expires_at = HLCTimestamp::new(
            now.physical + duration_ms,
            0,
        );
        self.renewal_count += 1;
    }

    /// Create a child lease delegated from this one
    pub fn delegate(&self, scope: LeaseScope, holder: String, now: HLCTimestamp) -> Lease {
        let mut child = Lease::new(
            scope,
            holder,
            now,
            // Child lease can't outlive parent
            self.time_remaining(now).unwrap_or(Duration::from_secs(0)),
        );
        child.parent_lease = Some(self.id);
        child
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lease_scope_covers() {
        let file_lease = LeaseScope::File(PathBuf::from("/data/file.txt"));
        assert!(file_lease.covers(&PathBuf::from("/data/file.txt")));
        assert!(!file_lease.covers(&PathBuf::from("/data/other.txt")));

        let dir_lease = LeaseScope::Directory {
            path: PathBuf::from("/data"),
            recursive: true,
        };
        assert!(dir_lease.covers(&PathBuf::from("/data/file.txt")));
        assert!(dir_lease.covers(&PathBuf::from("/data/subdir/file.txt")));
        assert!(!dir_lease.covers(&PathBuf::from("/other/file.txt")));

        let non_recursive = LeaseScope::Directory {
            path: PathBuf::from("/data"),
            recursive: false,
        };
        assert!(non_recursive.covers(&PathBuf::from("/data/file.txt")));
        assert!(!non_recursive.covers(&PathBuf::from("/data/subdir/file.txt")));
    }

    #[test]
    fn test_lease_specificity() {
        let root = LeaseScope::Directory {
            path: PathBuf::from("/"),
            recursive: true,
        };
        let data = LeaseScope::Directory {
            path: PathBuf::from("/data"),
            recursive: true,
        };
        let data_eu = LeaseScope::Directory {
            path: PathBuf::from("/data/eu"),
            recursive: true,
        };
        let file = LeaseScope::File(PathBuf::from("/data/eu/file.txt"));

        assert!(data.is_more_specific_than(&root));
        assert!(data_eu.is_more_specific_than(&data));
        assert!(file.is_more_specific_than(&data_eu));
        assert!(!root.is_more_specific_than(&data));
    }

    #[test]
    fn test_lease_expiration() {
        let now = HLCTimestamp::new(1000, 0);
        let lease = Lease::new(
            LeaseScope::File(PathBuf::from("/data/file.txt")),
            "node1".to_string(),
            now,
            Duration::from_secs(30),
        );

        assert!(!lease.is_expired(now));
        assert!(!lease.is_expired(HLCTimestamp::new(1000 + 29_999, 0)));
        assert!(lease.is_expired(HLCTimestamp::new(1000 + 30_000, 0)));
        assert!(lease.is_expired(HLCTimestamp::new(1000 + 30_001, 0)));
    }

    #[test]
    fn test_lease_renewal() {
        let now = HLCTimestamp::new(1000, 0);
        let mut lease = Lease::new(
            LeaseScope::File(PathBuf::from("/data/file.txt")),
            "node1".to_string(),
            now,
            Duration::from_secs(30),
        );

        assert_eq!(lease.renewal_count, 0);
        assert_eq!(lease.expires_at.physical, 31_000);

        // Renew at 25 seconds
        let renewal_time = HLCTimestamp::new(26_000, 0);
        lease.renew(renewal_time, Duration::from_secs(30));

        assert_eq!(lease.renewal_count, 1);
        assert_eq!(lease.expires_at.physical, 56_000); // 26s + 30s
    }
}
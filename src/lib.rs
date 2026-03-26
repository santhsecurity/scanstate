//! Generic checkpoint, journal, and progress primitives for scanners.
//!
//! # Quick Start
//!
//! ```rust
//! use scanstate::ScanCheckpoint;
//!
//! let mut checkpoint = ScanCheckpoint::new("my-scan-001");
//! checkpoint.mark_complete("https://target-1.com");
//! checkpoint.mark_complete("https://target-2.com");
//!
//! assert!(checkpoint.is_complete("https://target-1.com"));
//! assert!(!checkpoint.is_complete("https://target-3.com"));
//! assert_eq!(checkpoint.completed_count(), 2);
//! ```

#![warn(missing_docs)]
#![forbid(unsafe_code)]

/// Core checkpoint data structures for pausing and resuming workloads.
pub mod checkpoint;
/// Write-ahead transaction logging to prevent data loss on crashes.
pub mod journal;
/// Runtime metric and ETA calculators tracking exact real-time execution speeds.
pub mod progress;

pub use checkpoint::{CheckpointSettings, ScanCheckpoint, ScanStateError};
pub use journal::{Entry, WriteAheadJournal};
pub use progress::ScanProgress;

/// Trait for any scan state that can be checkpointed.
///
/// Implement this on your scanner's state to get free pause/resume.
pub trait Checkpointable {
    /// Mark a target as complete.
    fn mark_done(&mut self, target_id: &str);
    /// Check if a target is already done.
    fn is_done(&self, target_id: &str) -> bool;
    /// How many targets are done.
    fn done_count(&self) -> usize;
}

impl Checkpointable for checkpoint::ScanCheckpoint {
    fn mark_done(&mut self, target_id: &str) {
        self.mark_complete(target_id);
    }
    fn is_done(&self, target_id: &str) -> bool {
        self.is_complete(target_id)
    }
    fn done_count(&self) -> usize {
        self.completed_count()
    }
}

/// Load a checkpoint from file, or create a new empty one if the file doesn't exist.
pub fn load_or_new(path: impl AsRef<std::path::Path>, scan_id: &str) -> Result<checkpoint::ScanCheckpoint, ScanStateError> {
    match checkpoint::ScanCheckpoint::load(&path) {
        Ok(cp) => Ok(cp),
        Err(ScanStateError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(checkpoint::ScanCheckpoint::new(scan_id))
        },
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod adversarial_tests;

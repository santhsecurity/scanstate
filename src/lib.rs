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
///
/// This is a convenience function for resuming scans. If the checkpoint file exists,
/// it loads the saved state. Otherwise, it creates a fresh checkpoint with the given
/// scan ID.
///
/// # Parameters
///
/// - `path`: Path to the checkpoint file
/// - `scan_id`: Identifier for the scan (used only when creating a new checkpoint)
///
/// # Returns
///
/// Returns `Ok(ScanCheckpoint)` on success, or `Err(ScanStateError)` if the file
/// exists but cannot be read or parsed.
///
/// # Errors
///
/// - `ScanStateError::Io`: If the file exists but cannot be read
/// - `ScanStateError::Serde`: If the file contains invalid JSON
///
/// # Example
///
/// ```rust
/// use scanstate::load_or_new;
/// use std::io::Write;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let temp_dir = tempfile::tempdir()?;
/// # let checkpoint_path = temp_dir.path().join("checkpoint.json");
/// // Load existing checkpoint, or create new one if it doesn't exist
/// let mut checkpoint = load_or_new(&checkpoint_path, "my-scan")?;
///
/// // Mark some targets as complete
/// checkpoint.mark_complete("target-1");
/// checkpoint.mark_complete("target-2");
///
/// // Save for next time
/// checkpoint.save(&checkpoint_path)?;
/// # Ok(())
/// # }
/// ```
pub fn load_or_new(
    path: impl AsRef<std::path::Path>,
    scan_id: &str,
) -> Result<checkpoint::ScanCheckpoint, ScanStateError> {
    match checkpoint::ScanCheckpoint::load(&path) {
        Ok(cp) => Ok(cp),
        Err(ScanStateError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(checkpoint::ScanCheckpoint::new(scan_id))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod adversarial_tests;

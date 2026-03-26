//! Example showing how checkpointing can be used to track progress.
//!
//! Run: cargo run --example progress_bar

use scanstate::ScanCheckpoint;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let mut checkpoint = ScanCheckpoint::new("progress-scan");

    let targets = ["target_A", "target_B", "target_C"];

    for (i, target) in targets.iter().enumerate() {
        checkpoint.mark_complete(*target);
        println!("Progress: {}/{} completed.", i + 1, targets.len());
    }

    let path = dir.path().join("progress.json");
    checkpoint.save(&path).unwrap();
    println!("Saved final progress to disk.");
}

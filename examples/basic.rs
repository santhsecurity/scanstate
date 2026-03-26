//! Basic checkpoint usage — save and resume a scan.
//!
//! Run: cargo run --example basic

use scanstate::{Entry, ScanCheckpoint, WriteAheadJournal};

fn main() {
    let dir = tempfile::tempdir().unwrap();

    // Create a checkpoint for tracking progress
    let mut checkpoint = ScanCheckpoint::new("my-scan-001");

    // Simulate scanning targets
    let targets = vec!["https://a.com", "https://b.com", "https://c.com"];

    for target in &targets {
        if checkpoint.is_complete(target) {
            println!("  Skipping {} (already done)", target);
            continue;
        }

        println!("  Scanning {}...", target);
        // ... do actual scanning work ...

        checkpoint.mark_complete(*target);
    }

    // Save checkpoint to disk
    let path = dir.path().join("checkpoint.json");
    checkpoint.save(&path).unwrap();
    println!(
        "Saved checkpoint: {} targets complete",
        checkpoint.completed_count()
    );

    // Later: resume from checkpoint
    let resumed = ScanCheckpoint::load(&path).unwrap();
    println!(
        "Resumed: {} targets already done",
        resumed.completed_count()
    );

    // Journal for crash recovery
    let journal = WriteAheadJournal::new(dir.path().join("scan.journal"));
    journal
        .append(&Entry {
            target_id: "https://a.com".into(),
            status: "completed".into(),
            timestamp: 1234567890,
            findings_count: 3,
        })
        .unwrap();

    let entries = journal.replay().unwrap();
    println!("Journal has {} entries", entries.len());
}

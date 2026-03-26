# scanstate

Checkpoint a scan so you can resume it later. If the process crashes, pick up where you left off. Tracks which targets are done, writes a journal for crash recovery, and calculates ETA.

```rust
use scanstate::ScanCheckpoint;

let mut checkpoint = ScanCheckpoint::new("my-scan");
checkpoint.mark_complete("https://target-1.com");
checkpoint.mark_complete("https://target-2.com");
checkpoint.save("checkpoint.json").unwrap();

// Later, or after a crash:
let resumed = ScanCheckpoint::load("checkpoint.json").unwrap();
if resumed.is_complete("https://target-1.com") {
    // skip it
}
```

## Write-ahead journal

For crash recovery, append events to a journal. If the process dies mid-scan, replay the journal to rebuild state:

```rust
use scanstate::{WriteAheadJournal, Entry};

let journal = WriteAheadJournal::new("scan.journal");
journal.append(&Entry {
    target_id: "https://target-1.com".into(),
    status: "completed".into(),
    timestamp: 1234567890,
    findings_count: 3,
}).unwrap();

// After crash, replay:
let entries = journal.replay().unwrap();
// Or lenient replay (skips corrupt entries):
let (entries, corrupt_count) = journal.replay_lenient().unwrap();
```

## Progress tracking

```rust
use scanstate::ScanProgress;

let mut progress = ScanProgress::new(1000); // 1000 total targets
progress.record_completed();
progress.record_completed();
println!("ETA: {:?}", progress.eta());
println!("Rate: {:.1}/sec", progress.rate());
```

## Contributing

Pull requests are welcome. There is no such thing as a perfect crate. If you find a bug, a better API, or just a rough edge, open a PR. We review quickly.

## License

MIT. Copyright 2026 CORUM COLLECTIVE LLC.

[![crates.io](https://img.shields.io/crates/v/scanstate.svg)](https://crates.io/crates/scanstate)
[![docs.rs](https://docs.rs/scanstate/badge.svg)](https://docs.rs/scanstate)

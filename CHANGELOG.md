# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-03-26

### Added

- Added comprehensive documentation for all public functions
  - `load_or_new` in `lib.rs` with usage example
  - `ScanProgress::record_completed` with example
  - `ScanProgress::record_skipped` with example  
  - `ScanProgress::record_findings` with example
  - `ScanProgress::rate` with example
  - `ScanProgress::eta` with example

### Documentation

- All public functions now have complete rustdoc comments
- Added code examples to key functions for better discoverability
- Documented parameters and return values for all public APIs

## [0.1.0] - 2026-03-26

### Added

- Initial release of `scanstate`
- `ScanCheckpoint` for tracking completed scan targets with atomic persistence
- `WriteAheadJournal` for crash-recovery via append-only logging
- `ScanProgress` for runtime metrics and ETA calculations
- `Checkpointable` trait for pause/resume workflow integration
- `CheckpointSettings` for TOML-based configuration
- Comprehensive test suite including adversarial tests

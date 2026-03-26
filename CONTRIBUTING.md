# Contributing to scanstate (santh-checkpoint)

## How to set up dev environment
1. Ensure Rust is installed via `rustup`.
2. Clone the repository and navigate to this crate's directory.
3. Run `cargo build` to compile the library.

## How to run tests
- Execute `cargo test` to run all unit and integration tests.
- Execute `cargo test --examples` to verify that examples compile and run correctly.

## How to add a new feature
To add a new journal backend or progress serialization format, implement the relevant traits in `src/` and ensure atomic writes to prevent state corruption.

## Code style guidelines
- All code must be formatted with `cargo fmt`.
- Ensure `cargo clippy` produces no warnings (`cargo clippy -- -D warnings`).
- Use descriptive variable names and document public APIs.

## PR checklist
- [ ] Dev environment builds successfully.
- [ ] `cargo test` passes.
- [ ] Code is formatted and linted (`fmt` and `clippy`).
- [ ] New features are documented and tested.
- [ ] Relevant examples added or updated.

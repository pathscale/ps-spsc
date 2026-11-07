# Local prepublish checks

Before manually dispatching the publisher from `master`, review the change and run the applicable checks locally, including `cargo publish --dry-run`:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --release
cargo build --target thumbv7em-none-eabi
cargo +nightly miri test --lib
cargo publish --dry-run
```

The bare-metal target build verifies the crate's `#![no_std]` configuration; host tests alone link `std`. Miri covers the hand-written ring-buffer copies and deallocation paths. This crate defines no Cargo feature matrix and has no Loom tests, so there are no additional feature or Loom publication checks.

# Local prepublish checks

Before dispatching the manual publisher, review the exact commit and run these checks locally:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --release
cargo build --target thumbv7em-none-eabi
cargo +nightly miri test --lib
cargo publish --dry-run
```

The bare-metal target build verifies the crate's `#![no_std]` configuration; host tests alone link `std`. Miri covers the hand-written ring-buffer copies and deallocation paths. This crate defines no Cargo feature matrix and has no Loom tests, so there are no additional feature or Loom publication checks.

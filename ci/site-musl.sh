#!/bin/bash
# nicolas-ha.com's static linux-musl binary (task rust-site-no-container), built as `site image` builds it on the Mac
# (crates/site/src/image.rs: the same rustc, target, rust-lld and path remaps), then every route test of
# crates/site/tests/routes.rs run on it: Linux is the only place it runs before Fly. Not the Mac's bytes: on 2026-11-08
# the same source gave b6a6900e here and 939f186c there (one folder or another on the Mac gives the same bytes).
set -euo pipefail
rustup toolchain install 1.98.1 --profile minimal --target x86_64-unknown-linux-musl
src=$PWD
t=$PWD/target-musl
sysroot=$(rustc +1.98.1 --print sysroot)
commit=$(rustc +1.98.1 -vV | sed -n 's/^commit-hash: //p')
remaps=$(printf -- '--remap-path-prefix=%s=/cargo\x1f--remap-path-prefix=%s=/src\x1f--remap-path-prefix=%s=/target\x1f--remap-path-prefix=%s/lib/rustlib/src/rust=/rustc/%s' \
  "${CARGO_HOME:-$HOME/.cargo}" "$src" "$t" "$sysroot" "$commit")
CARGO_TARGET_DIR=$t CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld CARGO_ENCODED_RUSTFLAGS=$remaps \
  cargo +1.98.1 build --release --locked --target x86_64-unknown-linux-musl -p dash-site --bin site
bin=$t/x86_64-unknown-linux-musl/release/site
file "$bin"
sha256sum "$bin"
# the tests build for the host as usual: only the binary they drive is the musl one
SITE_BIN=$bin timeout 130 cargo nextest run --locked -p dash-site --no-fail-fast

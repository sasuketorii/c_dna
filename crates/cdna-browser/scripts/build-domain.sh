#!/bin/sh
set -eu
# Run this script through the repository development harness.
# The CLI version must match the exact wasm-bindgen dependency.
crate_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
repo_dir=$(CDPATH= cd -- "$crate_dir/../.." && pwd)
cargo build --manifest-path "$repo_dir/Cargo.toml" -p cdna-browser --target wasm32-unknown-unknown --release
test "$("$crate_dir/.tools/wasm-bindgen" --version)" = "wasm-bindgen 0.2.128"
"$crate_dir/.tools/wasm-bindgen" "$repo_dir/target/wasm32-unknown-unknown/release/cdna_browser.wasm" --target web --out-dir "$crate_dir/assets/domain" --out-name cdna_browser

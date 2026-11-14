#!/usr/bin/env bash
# Builds the browser engine into web/pkg.
#   ./build-wasm.sh            single-threaded, SIMD128 (stable Rust + wasm32 rust-std)
#   ./build-wasm.sh --threads  multithreaded (nightly + rust-src; needs COOP/COEP headers)
set -euo pipefail
cd "$(dirname "$0")"
target=wasm32-unknown-unknown
if [[ "${1:-}" == "--threads" ]]; then
  RUSTFLAGS="-C target-feature=+simd128,+atomics,+bulk-memory,+mutable-globals" \
    cargo +nightly build -p openpdn-wasm --release --target $target --features threads -Z build-std=std,panic_abort
else
  RUSTFLAGS="-C target-feature=+simd128" cargo build -p openpdn-wasm --release --target $target
fi
wasm-bindgen --target web --out-dir web/pkg --no-typescript target/$target/release/openpdn_wasm.wasm
if command -v wasm-opt >/dev/null; then
  wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    web/pkg/openpdn_wasm_bg.wasm -o web/pkg/openpdn_wasm_bg.wasm
fi
ls -la web/pkg/

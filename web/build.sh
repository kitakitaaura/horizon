#!/bin/sh
# Builds Horizon for the browser into web/pkg. Needs the horizon-web toolchain (web/toolchain.sh).
# Usage: web/build.sh [--release]
set -eu
cd "$(dirname "$0")/.."
profile=debug
flag=
if [ "${1:-}" = "--release" ]; then profile=release; flag=--release; fi
target=target/wasm32-unknown-unknown
# Cargo doesn't notice edits to the toolchain's std sources (web/std.patch): rebuild std when
# they change.
sys="$(rustc +horizon-web --print sysroot)/lib/rustlib/src/rust/library/std/src/sys"
digest="$(find "$sys" -type f -name '*.rs' | sort | xargs cat | shasum | cut -d' ' -f1)"
stamp="$target/$profile/.horizon-web-std"
if [ "$(cat "$stamp" 2>/dev/null)" != "$digest" ]; then
    rm -rf "$target/$profile/build/std" "$target/$profile/.fingerprint"/std-*
fi
# Incremental caches of the whole game run to several GB per profile.
CARGO_INCREMENTAL=0 cargo +horizon-web build -p horizon-web $flag \
    --target wasm32-unknown-unknown -Zbuild-std=std,panic_abort
mkdir -p "$target/$profile" && echo "$digest" > "$stamp"
wasm-bindgen --target web --out-dir web/pkg --no-typescript \
    "$target/$profile/horizon_web.wasm"

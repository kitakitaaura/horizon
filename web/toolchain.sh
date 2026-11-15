#!/bin/sh
# Makes the `horizon-web` rustup toolchain: the pinned nightly with std patched (web/std.patch)
# for Horizon's browser platform (a clock, threads on Web Workers, a filesystem in shared memory,
# environment variables, console output). Usage: web/toolchain.sh [destination]
set -eu
NIGHTLY=nightly-2026-11-13
here="$(cd "$(dirname "$0")" && pwd)"
dest="${1:-$here/../.local/horizon-web-toolchain}"
rustup toolchain install "$NIGHTLY" --profile minimal --component rust-src --target wasm32-unknown-unknown
src="$(rustc +"$NIGHTLY" --print sysroot)"
rm -rf "$dest"
mkdir -p "$(dirname "$dest")"
# A copy-on-write clone where the filesystem has them (APFS, btrfs), else a plain copy.
cp -cR "$src" "$dest" 2>/dev/null || cp -R "$src" "$dest"
(cd "$dest/lib/rustlib/src/rust/library" && patch -p1 < "$here/std.patch")
rustup toolchain link horizon-web "$dest"
echo "horizon-web toolchain ready: $(rustc +horizon-web --version)"

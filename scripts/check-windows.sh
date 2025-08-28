#!/usr/bin/env bash
set -euo pipefail

# Cross-check Windows build from Linux using the GNU (MinGW) toolchain.
# - Ensures target `x86_64-pc-windows-gnu` is installed
# - Verifies MinGW cross-compiler is available (x86_64-w64-mingw32-gcc)
# - Runs cargo clippy with warnings-as-errors for the Windows target
# - Builds release binaries for the Windows target

echo "[socktop] Windows cross-check: clippy + build (GNU target)"

have() { command -v "$1" >/dev/null 2>&1; }

if ! have rustup; then
  echo "error: rustup not found. Install Rust via rustup first (see README)." >&2
  exit 1
fi

if ! rustup target list --installed | grep -q '^x86_64-pc-windows-gnu$'; then
  echo "+ rustup target add x86_64-pc-windows-gnu"
  rustup target add x86_64-pc-windows-gnu
fi

if ! have x86_64-w64-mingw32-gcc; then
  echo "error: Missing MinGW cross-compiler (x86_64-w64-mingw32-gcc)." >&2
  if have pacman; then
    echo "Arch Linux: sudo pacman -S --needed mingw-w64-gcc" >&2
  elif have apt-get; then
    echo "Debian/Ubuntu: sudo apt-get install -y mingw-w64" >&2
  elif have dnf; then
    echo "Fedora: sudo dnf install -y mingw64-gcc" >&2
  else
    echo "Install the mingw-w64 toolchain for your distro, then re-run." >&2
  fi
  exit 1
fi

CARGO_FLAGS=(--workspace --all-targets --all-features --target x86_64-pc-windows-gnu)

echo "+ cargo clippy ${CARGO_FLAGS[*]} -- -D warnings"
cargo clippy "${CARGO_FLAGS[@]}" -- -D warnings

echo "+ cargo build --release ${CARGO_FLAGS[*]}"
cargo build --release "${CARGO_FLAGS[@]}"

echo "✅ Windows clippy and build completed successfully."


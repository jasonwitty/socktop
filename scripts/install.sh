#!/usr/bin/env bash
# Build socktop + socktop_agent from source and install them.
#
# Works on Linux (x86_64, arm64/armv7, riscv64) and macOS. Handles fresh
# installs and upgrades; if a systemd socktop-agent service is present, its
# binary is replaced in place and the service restarted.
#
#   ./scripts/install.sh                     # build HEAD of the repo you're in
#   ./scripts/install.sh --ref v1.60.0       # build a tag/branch (clones if needed)
#   ./scripts/install.sh --ref housekeeping-p2
#   ./scripts/install.sh --prefix ~/.local/bin --no-service
#
set -euo pipefail

REPO_URL="https://github.com/jasonwitty/socktop.git"
REF=""
PREFIX=""
NO_SERVICE=0
SRC_DIR="${SOCKTOP_SRC_DIR:-$HOME/.cache/socktop-src}"

while [ $# -gt 0 ]; do
  case "$1" in
    --ref)       REF="$2"; shift 2 ;;
    --prefix)    PREFIX="$2"; shift 2 ;;
    --no-service) NO_SERVICE=1; shift ;;
    -h|--help)   grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

say()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

OS="$(uname -s)"
ARCH="$(uname -m)"

# ---------- toolchain ----------
command -v git >/dev/null || die "git is required"
if ! command -v cargo >/dev/null; then
  # rustup may be installed but not on PATH in this shell
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null; then
  say "Rust toolchain not found — installing via rustup (stable, default profile)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi
command -v cc >/dev/null || warn "no C compiler found (apt: build-essential / brew: xcode-select --install) — the build may fail"
case "$ARCH" in
  riscv64*)
    # protoc-bin-vendored ships no riscv64 binary; the build falls back to
    # the system protoc (see build.rs).
    command -v protoc >/dev/null || die "riscv64 needs a system protoc: sudo apt install protobuf-compiler"
    ;;
esac

# ---------- source ----------
# If run from inside a socktop checkout and no --ref given, build that tree
# as-is (whatever is checked out, including local changes).
if [ -z "$REF" ] && git rev-parse --show-toplevel >/dev/null 2>&1 \
   && grep -qs '^name = "socktop"' "$(git rev-parse --show-toplevel)/socktop/Cargo.toml" 2>/dev/null; then
  SRC_DIR="$(git rev-parse --show-toplevel)"
  say "Building the current checkout: $SRC_DIR ($(git -C "$SRC_DIR" describe --always --dirty 2>/dev/null))"
else
  REF="${REF:-master}"
  if [ ! -d "$SRC_DIR/.git" ]; then
    say "Cloning $REPO_URL -> $SRC_DIR"
    git clone "$REPO_URL" "$SRC_DIR"
  fi
  say "Checking out $REF"
  git -C "$SRC_DIR" fetch --tags origin
  git -C "$SRC_DIR" checkout -q "$REF"
  # fast-forward when REF is a branch
  git -C "$SRC_DIR" merge --ff-only "origin/$REF" >/dev/null 2>&1 || true
fi

# ---------- build ----------
say "Building release binaries (this can take a while on SBCs)"
( cd "$SRC_DIR" && cargo build --release -p socktop -p socktop_agent )
CLIENT="$SRC_DIR/target/release/socktop"
AGENT="$SRC_DIR/target/release/socktop_agent"

# ---------- install ----------
if [ -z "$PREFIX" ]; then
  PREFIX="/usr/local/bin"
fi
SUDO=""
if [ ! -w "$PREFIX" ]; then
  if command -v sudo >/dev/null; then SUDO="sudo"; else
    PREFIX="$HOME/.local/bin"; mkdir -p "$PREFIX"
    warn "no sudo — installing to $PREFIX (ensure it is on your PATH)"
  fi
fi
say "Installing to $PREFIX"
$SUDO install -m 755 "$CLIENT" "$PREFIX/socktop"
$SUDO install -m 755 "$AGENT" "$PREFIX/socktop_agent"

# ---------- systemd service (Linux only) ----------
if [ "$OS" = "Linux" ] && [ "$NO_SERVICE" -eq 0 ] \
   && command -v systemctl >/dev/null \
   && systemctl list-unit-files 2>/dev/null | grep -q '^socktop-agent\.service'; then
  # The deb package's unit points at its own binary path; replace it in place
  # so the running service picks up this build.
  UNIT_BIN="$(systemctl show -p ExecStart socktop-agent.service 2>/dev/null \
              | sed -n 's/.*path=\([^ ;]*\).*/\1/p' | head -1)"
  if [ -n "$UNIT_BIN" ] && [ "$UNIT_BIN" != "$PREFIX/socktop_agent" ]; then
    say "Refreshing systemd service binary at $UNIT_BIN"
    $SUDO systemctl stop socktop-agent.service
    $SUDO install -m 755 "$AGENT" "$UNIT_BIN"
    $SUDO systemctl start socktop-agent.service
  else
    say "Restarting socktop-agent.service"
    $SUDO systemctl restart socktop-agent.service
  fi
  sleep 1
  systemctl --no-pager -l status socktop-agent.service | head -5 || true
fi

say "Installed:"
"$PREFIX/socktop" --version
"$PREFIX/socktop_agent" --version

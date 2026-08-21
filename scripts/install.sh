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

# The entire remainder runs inside main(), invoked on the LAST line. This
# makes the script safe against being MODIFIED WHILE RUNNING: when executed
# from the clone it manages, the git checkout below replaces this very file,
# and bash reads scripts lazily by byte offset — without this wrapper it
# resumes parsing the NEW file at the OLD offset and executes an arbitrary
# tail of it (observed: the fresh-service path ran on a host whose unit
# already existed). With main(), the whole script is parsed before any of
# it executes.
main() {

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

# Update every other copy on PATH as well. A stale `cargo install` in
# ~/.cargo/bin would otherwise SHADOW the fresh binary (~/.cargo/bin
# usually precedes /usr/local/bin on PATH), leaving `socktop --version`
# stuck on the old release after a "successful" install.
update_path_copies() {
  local name="$1" src="$2" copy dir
  # type -ap lists every match on PATH (bash builtin, symlinks not resolved)
  for copy in $(type -ap "$name" | sort -u); do
    [ "$copy" = "$PREFIX/$name" ] && continue
    dir="$(dirname "$copy")"
    say "Updating additional copy on PATH: $copy"
    if [ -w "$copy" ] || [ -w "$dir" ]; then
      install -m 755 "$src" "$copy"
    else
      # Non-fatal: an un-updatable extra copy shouldn't kill the install,
      # but the user must know it may shadow the fresh binary.
      $SUDO install -m 755 "$src" "$copy"         || warn "could not update $copy — it may shadow $PREFIX/$name"
    fi
  done
}
update_path_copies socktop "$CLIENT"
update_path_copies socktop_agent "$AGENT"

# ---------- systemd service (Linux only) ----------
# System-level operations (unit files, users, service control) need root no
# matter where the binaries were installed — decide independently of PREFIX.
SYS_SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null; then SYS_SUDO="sudo"; else SYS_SUDO="__none__"; fi
fi
if [ "$SYS_SUDO" = "__none__" ] && [ "$NO_SERVICE" -eq 0 ]; then
  warn "no sudo available — skipping systemd service management"
  NO_SERVICE=1
fi
if [ "$OS" = "Linux" ] && [ "$NO_SERVICE" -eq 0 ] && command -v systemctl >/dev/null; then
  if systemctl cat socktop-agent.service >/dev/null 2>&1; then
    # UPGRADE: the unit file is the operator's (SSL, tokens, ports may be
    # configured there) — never overwrite it. Only the binary it points at
    # is replaced, then the service is restarted.
    say "Existing socktop-agent.service found — preserving unit file, refreshing binary"
    UNIT_BIN="$(systemctl show -p ExecStart socktop-agent.service 2>/dev/null \
                | sed -n 's/.*path=\([^ ;]*\).*/\1/p' | head -1)"
    if [ -n "$UNIT_BIN" ] && [ "$UNIT_BIN" != "$PREFIX/socktop_agent" ]; then
      $SYS_SUDO systemctl stop socktop-agent.service
      $SYS_SUDO install -m 755 "$AGENT" "$UNIT_BIN"
      $SYS_SUDO systemctl start socktop-agent.service
    else
      $SYS_SUDO systemctl restart socktop-agent.service
    fi
  else
    # FRESH INSTALL: unit + the system user it runs as + its state dir,
    # then enable and start. Mirrors the deb package's postinst and
    # https://www.socktop.io/assets/docs/installation/agent-service.html
    say "No socktop-agent.service found — installing and enabling it"

    if ! getent group socktop >/dev/null; then
      $SYS_SUDO groupadd --system socktop
    fi
    if ! getent passwd socktop >/dev/null; then
      NOLOGIN="$(command -v nologin || echo /usr/sbin/nologin)"
      $SYS_SUDO useradd --system -g socktop -d /var/lib/socktop -M -s "$NOLOGIN" socktop
    fi
    $SYS_SUDO mkdir -p /var/lib/socktop
    $SYS_SUDO chown socktop:socktop /var/lib/socktop
    $SYS_SUDO chmod 755 /var/lib/socktop

    UNIT_TMP="$(mktemp)"
    if [ -f "$SRC_DIR/docs/socktop-agent.service" ]; then
      cp "$SRC_DIR/docs/socktop-agent.service" "$UNIT_TMP"
    else
      # Fallback for refs that predate docs/socktop-agent.service
      cat > "$UNIT_TMP" <<'UNIT'
[Unit]
Description=Socktop agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/socktop_agent --port 3000
Environment=RUST_LOG=info
# Optional auth:
# Environment=SOCKTOP_TOKEN=changeme
# TLS (self-signed cert on first run, default port 8443):
# Environment=SOCKTOP_ENABLE_SSL=1
Restart=on-failure
User=socktop
Group=socktop
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
    fi
    # Pick the agent port: 3000 by default, but NEVER bind onto a port that
    # something else already holds (e.g. Gitea/Umami and friends love 3000)
    # — that puts the fresh service straight into a crash-restart loop.
    AGENT_PORT=""
    for p in 3000 3001 3010 3231 3232; do
      if ! ss -tln 2>/dev/null | awk '{print $4}' | grep -q ":${p}\$"; then
        AGENT_PORT="$p"
        break
      fi
    done
    if [ -z "$AGENT_PORT" ]; then
      AGENT_PORT=3000
      warn "no free port among the defaults — using 3000; edit the unit if the service fails to start"
    elif [ "$AGENT_PORT" != "3000" ]; then
      warn "port 3000 is already in use by another service — configuring the agent on port $AGENT_PORT"
    fi

    # Point ExecStart at wherever this run installed the agent, on the chosen port.
    sed -i.bak -e "s|^ExecStart=[^ ]*socktop_agent|ExecStart=$PREFIX/socktop_agent|" \
               -e "s|--port [0-9]*|--port $AGENT_PORT|" "$UNIT_TMP"
    rm -f "$UNIT_TMP.bak"

    $SYS_SUDO install -o root -g root -m 0644 "$UNIT_TMP" /etc/systemd/system/socktop-agent.service
    rm -f "$UNIT_TMP"
    $SYS_SUDO systemctl daemon-reload
    $SYS_SUDO systemctl enable --now socktop-agent.service
    say "Service installed — agent URL: ws://$(hostname):$AGENT_PORT/ws"
    say "To enable TLS or a token, edit /etc/systemd/system/socktop-agent.service, then: sudo systemctl daemon-reload && sudo systemctl restart socktop-agent"
  fi
  sleep 1
  systemctl --no-pager -l status socktop-agent.service | head -5 || true
fi

say "Installed:"
"$PREFIX/socktop" --version
"$PREFIX/socktop_agent" --version
say "Active on PATH: $(type -p socktop || true) / $(type -p socktop_agent || true)"
socktop --version

}

# exit in the same parse unit as the call: after main returns, bash must not
# read another byte from this (possibly replaced) file.
main "$@"; exit $?

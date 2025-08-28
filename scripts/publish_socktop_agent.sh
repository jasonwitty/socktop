#!/usr/bin/env bash
set -euo pipefail

# Publish job: "publish new socktop agent version"
# Usage: ./scripts/publish_socktop_agent.sh <new_version>

if [[ ${1:-} == "" ]]; then
  echo "Usage: $0 <new_version>" >&2
  exit 1
fi

NEW_VERSION="$1"
ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
CRATE_DIR="$ROOT_DIR/socktop_agent"

echo "==> Formatting socktop_agent"
(cd "$ROOT_DIR" && cargo fmt -p socktop_agent)

echo "==> Running tests for socktop_agent"
(cd "$ROOT_DIR" && cargo test -p socktop_agent)

echo "==> Running clippy (warnings as errors) for socktop_agent"
(cd "$ROOT_DIR" && cargo clippy -p socktop_agent -- -D warnings)

echo "==> Building release for socktop_agent"
(cd "$ROOT_DIR" && cargo build -p socktop_agent --release)

echo "==> Bumping version to $NEW_VERSION in socktop_agent/Cargo.toml"
sed -i.bak -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$NEW_VERSION\"/" "$CRATE_DIR/Cargo.toml"
rm -f "$CRATE_DIR/Cargo.toml.bak"

echo "==> Committing version bump"
(cd "$ROOT_DIR" && git add -A && git commit -m "socktop_agent: bump version to $NEW_VERSION")

CURRENT_BRANCH=$(cd "$ROOT_DIR" && git rev-parse --abbrev-ref HEAD)
echo "==> Pushing to origin $CURRENT_BRANCH"
(cd "$ROOT_DIR" && git push origin "$CURRENT_BRANCH")

echo "==> Publishing socktop_agent $NEW_VERSION to crates.io"
(cd "$ROOT_DIR" && cargo publish -p socktop_agent)

echo "==> Done: socktop_agent $NEW_VERSION published"


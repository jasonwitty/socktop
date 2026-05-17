#!/usr/bin/env bash
set -euo pipefail

# Sync this repo to the 'gitea' remote as a mirror.
# - Mirrors ALL refs (branches, tags) and prunes removed ones.
# - This makes the Gitea repo match GitHub exactly.

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Error: not inside a git repo" >&2
  exit 1
fi

if ! git remote get-url gitea >/dev/null 2>&1; then
  echo "Missing 'gitea' remote. Add it with:" >&2
  echo "  git remote add gitea https://gt.wittyoneoff.com/jason/socktop.git" >&2
  exit 1
fi

echo "Fetching from origin (pruning)..."
git fetch origin --prune --tags

echo "Pushing mirror to gitea..."
git push gitea --mirror

echo "Done: Gitea should now match origin (GitHub)."


<!--
Rough shape the recent PRs (#42, #43, #45) settled on. Trim sections that don't apply;
this is a prompt, not a form to fill rigidly.
-->

## Summary

One paragraph: what this changes and why it matters.

## What each change actually does

For anything security- or dependency-shaped, say whether the flagged path is **reachable in this
tree** and how it was checked, not just which version was bumped. A table works well when there is
more than one advisory.

## Verification

The commands that were run and what they printed — including the ones that need a live agent:

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --workspace
cargo test --workspace
```

Remember there are three lockfiles/workspaces (`Cargo.lock`, `zellij_socktop_plugin/`,
`socktop_wasm_test/`) — see [AGENTS.md](../AGENTS.md).

## Notes for review

- Pre-existing things deliberately left alone (and why).
- Follow-ups this PR sets up but does not do.

---

Written with lovey: https://gt.wittyoneoff.com/jason/lovey

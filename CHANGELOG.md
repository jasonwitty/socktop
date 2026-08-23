# Changelog

## 1.60.0 — unreleased

Everything since `v1.50.0`. Applies to all three crates (`socktop`, `socktop_agent`, `socktop_connector`), which move to 1.60.0 together.

### Security

- **Certificate pinning is now real.** With `--verify-hostname` off (the default), the client previously accepted *any* server certificate — the `--tls-ca` file was never consulted. The presented certificate must now be byte-identical to one in the pinned PEM (multi-cert files supported for rotation). If you use TLS, update the client: earlier versions are MITM-able despite the pinning documentation. (housekeeping-p2)
- `key.pem` is created with mode 0600 (was world-readable 0644); agents also tighten existing keys on startup. (housekeeping-p2)
- The agent's per-PID caches now evict (60s age / 64 entries); previously they grew without bound. (housekeeping-p2)

### Performance

- Agent CPU on GPU machines cut ~6× (measured 23.5 → 4.0 ms/s at default polling): GPU collection moved to a dedicated worker thread that keeps the NVML session open instead of re-initializing it every 1.5 s on the async runtime. (housekeeping-p2)
- `journalctl` no longer blocks the agent's async workers. (housekeeping-p2)
- Cached "no temp sensor / no GPU" results count as fresh — no more per-request rescans on hosts without them. (housekeeping-p2)
- Nagle disabled on all connection paths (small request/response frames). (housekeeping-p2)

### TUI

- **Compact layout for small windows**: when the window is too short for the Disks pane, Disks is dropped, Memory/Swap go side by side, GPU collapses to one line (omitted if absent), and the reclaimed rows keep the CPU graph and per-core bars visible. `--compact` pins it. (#37)
- **Width-aware text**: header, CPU title, and process table shed detail by priority as the terminal narrows instead of overwriting each other; process Name column is now the last to go, not the first. Fixed sort-header clicks landing up to 4 columns off. (#38)
- **Responsive input**: keys and mouse are handled within ~30 ms instead of queueing for a full metrics interval. (housekeeping-p2)
- **No more freezes**: all requests carry a 5 s timeout; a dead connection shows the reconnect modal (with working `q`) instead of hanging the UI. Consecutive timeouts surface a persistent "agent not responding" error. (housekeeping-p2)
- Old agents without the per-process endpoints once again show "Agent Update Required" instead of a reconnect loop. (housekeeping-p2)
- Journal pane distinguishes "no entries" from "no journal access" (e.g. user-run/demo agents) and shows journalctl's hint plus the fix. (housekeeping-p2)
- Scatter-plot axes align correctly for large CPU-time values. (housekeeping-p2)
- Demo mode explains how to install `socktop_agent` when the binary is missing. (#36)

### Correctness

- Process/child CPU times were sent as ms but displayed as µs — values rendered 1000× too small in the details modal. (housekeeping-p2)
- Non-Linux per-process CPU% no longer truncates multi-core usage (clamp after divide). (housekeeping-p2)
- Journal timestamps are real RFC 3339 UTC with numeric sorting (additive `timestamp_us`). (housekeeping-p2)
- Partition detection uses `/sys/block` on Linux — whole-disk filesystems (`nvme0n1`, `zram1`) are no longer misclassified as partitions. (housekeeping-p2)
- Network rates use agent-side sample timestamps (additive `sampled_at_ms`), eliminating rate sawtooth from TTL-cached snapshots; falls back to the client clock with older agents. (housekeeping-p2)
- The details modal's Command/exe/cwd fields are populated again (dropped by an earlier refresh optimization). (housekeeping-p2)
- Non-ASCII device names no longer panic the disk pane. (housekeeping-p2)

### Wire format (additive only — old/new client-agent pairs keep working)

- `Metrics.sampled_at_ms` (epoch ms of actual collection)
- `JournalEntry.timestamp_us` (epoch µs), `JournalEntry.timestamp` now RFC 3339
- `JournalResponse.notice` (journal-access hint)

### Internal / packaging

- ratatui 0.28 → 0.30 (#33); aws-lc-rs advisories patched (#34); Debian packaging for the agent (#25); assorted dependabot bumps.
- ~3,100 lines of dead code removed, including an orphaned pre-refactor copy of the connector.
- `socktop` consumes `socktop_connector` via a path+version dep — connector changes are testable in-repo before publishing.
- wasm examples build against the in-repo connector; note `zellij_socktop_plugin` has pre-existing compile errors and needs its own rework.

### Process kill (PR #40)

- **Kill a local process from the TUI** (`t` on a selected process, or inside Process Details): btop-style Terminate/Force-kill confirmation. Local agents only — the signal is sent by socktop itself with its own privileges, never over the wire; remote agents never show the option. PID-reuse guarded (the confirmed name must still own the PID at signal time).
- **Agent no longer reports dead processes**: a long-lived sysinfo `System` accumulated every process ever seen (21k+ entries on a 289-process host), inflating memory, per-poll work, and the process count — and keeping killed processes on screen forever. Update agent and client together on machines where the kill feature will be used.
- Killed rows leave the list when the process actually exits and cannot be resurrected by cached agent snapshots; details views for dead processes close themselves, including through parent-navigation chains.
- Selection hint no longer vanishes for long process names; confirmation/info dialogs size to their content.

### Upgrade notes

- **Release/publish order**: `socktop_connector` → `socktop` → agent packages.
- Clients older than 1.60 work against 1.60 agents and vice versa; the security fix is client-side, so prioritize client updates where TLS is used.

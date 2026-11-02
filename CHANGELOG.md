# Changelog

## Unreleased

### Security

- **rustls 0.23.35 → 0.23.46**, which closes three advisories that all applied
  to the pinned version: GHSA-c8m5-c952-6cm4 / RUSTSEC-2026-0346 (`Debug` on
  client session values leaked the TLS 1.2 master secret / TLS 1.3 resumption
  PSK), GHSA-h2v8-394m-p7vv / RUSTSEC-2026-0348 (Brotli-encoded certificates
  could force a 1 GiB allocation before authentication), and
  GHSA-w44r-93j7-qv28 / RUSTSEC-2026-0349 (an unauthenticated `ServerHello`
  poisons the per-server-name key-exchange prediction). The first two are not
  reachable in socktop as configured — no custom session store, `brotli` feature
  off — but the third is: rustls enables an in-memory session store by default,
  and the pinned-CA client used it.
- **TLS session resumption is now disabled in the `wss://` client**
  (`socktop_connector`), independently of the rustls version. Every
  `connect_to_agent()` is a fresh session, so the store only ever cost memory
  (master secrets and resumption PSKs kept for the process lifetime) and
  carried the poisoned key-share prediction above; the connector now opts out
  with `Resumption::disabled()`.
- Pulled the remaining RustSec advisories out of the lockfiles — they were not
  surfaced as Dependabot alerts at all: `tokio` 1.48 → 1.54.1
  (RUSTSEC-2026-0386), `h2` 0.4.12 → 0.4.16 (RUSTSEC-2026-0258, unbounded empty
  DATA frames on the agent's HTTP stack), `anyhow` 1.0.100 → 1.0.103
  (RUSTSEC-2026-0190), `ratatui-core` 0.1.0 → 0.1.2 to get past `lru` 0.16
  (RUSTSEC-2026-0253, `LruCache::pop()` use-after-free), `libz-sys` in the
  Zellij plugin (RUSTSEC-2026-0418) and `anyhow` in `socktop_wasm_test`. All
  three lockfiles are now clean apart from the informational "rustls-pemfile is
  unmaintained" notice, which CI pins as an allowed informational warning.
- **CI now fails on advisories**: a `cargo audit` job runs over all three
  lockfiles (`Cargo.lock`, `zellij_socktop_plugin/`, `socktop_wasm_test/`),
  plus a `.github/dependabot.yml` covering the workspace, both standalone wasm
  workspaces and the workflow pins.
- `thiserror` 2.0.17 → 2.0.21: clippy on Rust 1.99 turned the `#[from] source:`
  fields of the connector's error enum into `redundant_field_names` errors, so
  the existing `clippy -- -D warnings` gate was red on master.

### Zellij plugin

- `zellij-tile` 0.40 → 0.45. The old SDK dragged in `surf → http-types → rand`
  0.7 and `clap` 3 → `atty`, both flagged and both unfixable inside 0.40;
  0.45 dropped `surf` and moved to clap 4, so they are simply gone.
- **The plugin now compiles.** It hadn't for a while: `register_plugin!` has
  emitted its own `STATE` for ages, so the hand-rolled `static mut STATE` was a
  duplicate definition, `BTreeMap`/`Key` no longer come from the prelude, and it
  read a `Metrics::load_avg_1m` field that does not exist. State now lives in
  `self` (no `static mut`), key handling uses `BareKey`, and the build target is
  `wasm32-unknown-unknown` (not `wasm32-wasi`, which Zellij never used).
  A CI job builds it so SDK drift can't rot it again. Plugins built against
  0.45 need Zellij 0.45.

### TUI

- **`--no-kill` flag and `SOCKTOP_NO_KILL` env var** disable the local
  process-kill feature regardless of agent locality, for shared terminals and
  public demos (e.g. the socktop.io webterm). Either one forces the feature
  off and suppresses the `t` kill hints; the env var covers every socktop
  invocation under a deployment without touching command lines. `App`'s
  builder renamed `with_local` → `with_kill_enabled` to match what it now
  means (locality fact AND policy).

## 1.60.1 — unreleased

Identical to 1.60.0 plus rebuilt Debian packages: the 1.60.0 debs were linked
against glibc 2.39 (a GitHub runner migration) and would not install on
Debian 12 / Raspberry Pi OS bookworm. CI now pins the build environment and
gates every package against the fleet's glibc floor. 1.60.0 was never
published to crates.io.

Everything since `v1.50.0`. Applies to all three crates (`socktop`, `socktop_agent`, `socktop_connector`), which move to 1.60.1 together.

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

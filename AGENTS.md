# AGENTS.md — working notes for socktop

Work in progress. Everything below was verified on 2026-11-02 (rustc/clippy/rustfmt 1.99.1, Linux)
unless it is marked **open question**. Add to this file when something durable is learned; fix it when
it rots. It is context for whoever (human or agent) picks the repo up next, not a spec.

## Layout

- Workspace members: `socktop` (the TUI), `socktop_agent`, `socktop_connector`.
- **Not** workspace members: `zellij_socktop_plugin/` and `socktop_wasm_test/` are standalone crates
  with their own `[workspace]` and their own `Cargo.lock`. Anything that builds, audits or updates
  dependencies has to be run three times — `find . -name Cargo.lock` (outside `target/`) is the check.
- wasm target is `wasm32-unknown-unknown`, **not** `wasm32-wasi` (older docs said otherwise).

## The real gates (mirror `.github/workflows/ci.yml`)

Run in each of the three workspaces separately:

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --workspace
cargo test --workspace
```

- `security` job: `rustsec/audit-check` over all three lockfiles, with `RUSTSEC-2025-0134`
  (rustls-pemfile unmaintained, no fixed version exists) pinned in `ignore:`.
- `plugin` job: builds the plugin wasm and `cargo check`s `socktop_wasm_test`. The main matrix never
  compiles the plugin at all — that is exactly how its SDK drift went unnoticed for months.
- There is also `build-deb.yml` (four cross-compiled .deb jobs) which runs on PRs too.

## Things that cost time to rediscover

- **clippy 1.99 + thiserror <= 2.0.17 breaks `-D warnings` on master.** `#[from] source: E` expands to
  `{ source: source }`, so `redundant_field_names` fires in *generated* code
  (`socktop_connector/src/error.rs`). Any PR off master is red until thiserror >= 2.0.21. A clean
  clippy run from scratch is the only way to see it; older clippy is silent.
- **`wss://` requires a pinned CA.** `tokio-tungstenite` is built with `__rustls-tls`, so no webpki
  root store is compiled in: without `--tls-ca`/`SOCKTOP_TLS_CA` a wss connection fails. That failure
  is expected, not a regression.
- Manual TLS end-to-end check (exercises the connector's TLS path, which CI does not):
  `SOCKTOP_ENABLE_SSL=1 SOCKTOP_AGENT_GPU=0 SOCKTOP_AGENT_TEMP=0 ./target/release/socktop_agent -p 3443`
  (self-signed cert lands in `~/.config/socktop_agent/tls/cert.pem`), then
  `SOCKTOP_WS=wss://127.0.0.1:3443/ws SOCKTOP_TLS_CA=<that cert> cargo test -p socktop_connector --test integration_test`.
  Without `SOCKTOP_TLS_CA` the probe hits `ws://` only.
- The wss client deliberately sets `Resumption::disabled()` (GHSA-w44r-93j7-qv28: an unauthenticated
  ServerHello poisons rustls' per-server-name key-share prediction, which silently drops the
  post-quantum X25519MLKEM768 share on later handshakes). Don't re-enable resumption without
  re-reading that advisory.
- **Dependabot does not see every RustSec advisory.** An OSV scan
  (`POST https://api.osv.dev/v1/querybatch`, `{"queries":[{"package":{"name":..,"ecosystem":"crates.io"},"version":..}]}`,
  results under `vulns`, batches of ~100) over the three lockfiles found tokio/h2/anyhow/lru/libz-sys
  advisories that had no alert at all. Run it before claiming "no advisories left".
- Dependabot alerts close when the fixed lockfile reaches the **default branch**; citing the GHSA id in
  a commit message makes the link explicit.
- **`docs/PERFORMANCE_REVIEW.md` and `docs/SECURITY_REVIEW.md` are deliberately not in the repo** while
  their findings are open (the repo is public; S1 and S3 describe reachable unpatched behaviour).
  Commit messages and PR bodies cite findings by id — P1..P12 for performance, S1..S10 for security. If a
  comment in a tracked file wants to cite one, describe the constraint inline instead of pointing at a
  path that is not in the tree.
- Zellij plugin specifics (0.45 SDK): needs a Zellij 0.45.x runtime; `register_plugin!` already emits
  the `thread_local STATE` (hand-rolling a second one is a compile error *and* was unsound as a
  `static mut`); `Event::Key` carries `KeyWithModifier { bare_key, key_modifiers }`, not the old `Key`
  enum; there is no `Metrics::load_avg_1m`; `uuid` needs its `js` feature for wasm randomness. The
  connector call is driven by `futures::executor::block_on` — a known limitation of the scaffold.
- Native `cargo check` inside `zellij_socktop_plugin/` fails here while building `openssl-sys`
  (`Can't locate FindBin.pm`). Only the wasm target is meaningful for that crate.

## Performance (all numbers from a Pi 5: Debian 13, aarch64, 4 cores, ~200 processes)

- Reproduce with `scripts/run_perf.sh <agent-binary> <label>` — it drives `scripts/perf_probe.py`, a
  stdlib-only WS client (no pip, no venv) with four modes: `cadence` (one client at the TUI's default
  polling, 40 s), `detail` (per-PID endpoints, cold and within TTL), `flood` (10 clients on
  `get_processes`, 10 s), `tls` (n handshakes + upgrades; set `SOCKTOP_PERF_CA=<cert>` to verify the way
  `--tls-ca` does). It reads the agent's own `/proc` entries; `/proc/<pid>/io` (the syscall column) needs
  the same user or root and prints n/a otherwise.
- **A TTL cache hit must be a refcount bump.** The agent's caches hold
  `CacheEntry<axum::extract::ws::Message>` — the encoded wire frame, gzip included — not the decoded
  payload. `respond_frame` in `ws.rs` is the one shape: TTL hit = clone a `Message`, miss = collect +
  encode once + store. Caching the decoded payload meant every hit re-cloned N `String`s, re-encoded
  protobuf and re-gzipped: 121.7 us -> 14.5 us per `get_processes` reply.
- **sysinfo:** `without_tasks()` stays in the refresh (task maps cost a syscall each). Per-PID CPU is a
  diff of `accumulated_cpu_time()` against `ProcCpuTracker.last_per_pid`; a changed value that is *lower*
  than last time means the PID was reused. Child enumeration walks `system.processes()` once via
  `proc.parent()` — do not reintroduce a second `/proc` scan. `get_processes` costs 692 read syscalls,
  was 1574.
- **The agent's runtime is `current_thread` by default.** Every request path funnels through locks one
  poll already holds; the slow collectors (journalctl, NVML) use `spawn_blocking`, which works on a
  current_thread runtime. `SOCKTOP_WORKER_THREADS=N` opts back into a multi-thread runtime, so
  `rt-multi-thread` must stay in the agent's tokio feature list even though the default never uses it.
- **Crypto: `ring` only, everywhere.** `cargo tree -i aws-lc-rs` must match nothing in all three
  lockfiles; the agent's `axum-server` is `tls-rustls-no-provider` and installs
  `crypto::ring::default_provider()` explicitly. The cost is that `wss://` negotiates no post-quantum
  X25519MLKEM768 share (the client was already ring, so it never negotiated one anyway). To get PQ back,
  enable aws-lc-rs in the connector *and* the agent — one side alone changes nothing. Measured on the Pi:
  ring handshakes are about 2x cheaper than aws-lc (2.8 ms vs 5.6 ms, 356/s vs 180/s).
- **Workspace `tokio` carries no features** (`Cargo.toml`); each member lists its own. A workspace build
  unifies features and hides a missing one — verify with `cargo build -p <member>`.
- **The arm64 `.deb` is built `--no-default-features`.** `gfxinfo` links libdrm/libdrm_amdgpu at build
  time, so a GPU-enabled arm64 package Depends on two libraries no Pi uses and runs the GPU probe on the
  first process-detail request. aarch64 with a real GPU (Jetson) is a local build with the feature.
- Building on a Pi over ssh: `export PATH=$HOME/.cargo/bin:$PATH` — a non-login shell has no cargo.
- **Measured and left alone** (don't re-litigate without new data): dropping sysinfo from the TUI
  (−111 KB binary, cold `cargo build -p socktop` 30.9 s vs 31.2 s — no build-time win, and the kill path
  would need hand-written unsafe Linux/other-unix/Windows code in CI); removing axum/hyper/tower from the
  agent (plausibly the biggest remaining win, entirely unmeasured — spike it before committing); the TUI's
  double draw per tick and 33 ms input poll (0.17% of a core); the 1 s hwmon rescan (100–150 reads/s).

## PR conventions

- Update `CHANGELOG.md` under `## Unreleased`.
- Commit subject: imperative, what changed. Body: *why*, plus the verification actually run (commands +
  results). Reference GHSA/RUSTSEC ids in the body when fixing advisories.
- PR body ends with the attribution footer:
  `Written with lovey: https://gt.wittyoneoff.com/jason/lovey`
  (pre-filled by `.github/PULL_REQUEST_TEMPLATE.md`; see #42, #43, #45).
- Don't fold unrelated fixes into a security PR without saying why — but do fix a red shared gate, since
  nothing else can merge through it.

## Open questions (not verified yet)

- Should the zellij plugin ship in release artifacts? Nothing packages it today, so SDK drift is cheap
  but so is the breakage.
- Per-process GPU attribution ("which card is llama.cpp on?"): `nvml-wrapper` 0.10 exposes no
  `*_running_processes()`; needs raw `nvml-wrapper-sys`. Deliberately not started.
- ratatui's `Gauge` renders its centred label even when the span is empty, blanking one cell in the
  middle of every bar (`█████ ████`). Repo-wide, pre-existing, not yet triaged.
- Whether `docs/` and `CHANGELOG.md` should carry the same security narrative in different shapes.

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
- Zellij plugin specifics (0.45 SDK): needs a Zellij 0.45.x runtime; `register_plugin!` already emits
  the `thread_local STATE` (hand-rolling a second one is a compile error *and* was unsound as a
  `static mut`); `Event::Key` carries `KeyWithModifier { bare_key, key_modifiers }`, not the old `Key`
  enum; there is no `Metrics::load_avg_1m`; `uuid` needs its `js` feature for wasm randomness. The
  connector call is driven by `futures::executor::block_on` — a known limitation of the scaffold.
- Native `cargo check` inside `zellij_socktop_plugin/` fails here while building `openssl-sys`
  (`Can't locate FindBin.pm`). Only the wasm target is meaningful for that crate.

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

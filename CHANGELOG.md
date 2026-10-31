# Changelog

## Unreleased

### TUI

- **Memory and Swap readouts gained a digit** — `9.63GB / 46.93GB`,
  `cache 30.35GB`, `0B / 8.00GB` — matching btop's precision so the two can be
  compared side by side, and so the last digit visibly ticks between refreshes
  instead of the numbers looking frozen. Tables, disks and network rates stay
  at one decimal; the unit thresholds are shared, so the two precisions cannot
  disagree about where KB becomes MB.
- **The Memory bar is now three stacked segments** — used (magenta), reclaimable
  cache (blue) and free — instead of one bar over a blank track. `used` has
  always been `MemTotal - MemAvailable`, i.e. the kernel counts clean
  file-backed pages as available, not used; a process whose RSS is mostly a
  mapped file (llama.cpp mmaps its GGUF by default) therefore shows gigabytes
  in the process list while the bar barely moves. That was correct but looked
  like a bug, and now the reclaimable slice is visible and labelled in place
  (`cache 28.10GB`, `free 8.50GB`), with `9.40GB / 46.00GB` moved to the pane title
  so it stays readable in the compact layout. Agents that do not send the new
  fields, and Windows (where free == available), fall back to the old look.
  `scripts/mem_accounting_check.sh` reproduces both sides of the accounting
  against a live agent: an 8 GiB anonymous heap moves the bar the whole way, a
  1.6 GiB mmap'd file moves the process row and not the bar.
- **`--no-kill` flag and `SOCKTOP_NO_KILL` env var** disable the local
  process-kill feature regardless of agent locality, for shared terminals and
  public demos (e.g. the socktop.io webterm). Either one forces the feature
  off and suppresses the `t` kill hints; the env var covers every socktop
  invocation under a deployment without touching command lines. `App`'s
  builder renamed `with_local` → `with_kill_enabled` to match what it now
  means (locality fact AND policy).

- **The GPU pane shows every card, not just one.** A 2×3090 llama.cpp host
  previously reported a single card. The pane now carries a tab strip —
  `All | 0:RTX 3090 | 1:RTX 3090` — switched with `g`/`G`, `Tab`/`Shift-Tab`, a
  click on a tab, or the wheel anywhere over the pane; compact mode has no strip,
  so a click there cycles. The aggregate tab sums VRAM (that is the number that
  tells you a layer failed to fit) but takes the **max** for utilisation and
  temperature rather than the average: NVML utilisation is a per-device duty cycle
  over a one-second window, so a pegged card plus an idle one would average to a
  healthy-looking 50% that neither card is in. The per-card line
  (`0 87% 18.4GB 68°   1 3% 2.1GB 41°`) stays visible so nothing is hidden behind
  the aggregate: card index, that card's utilisation, that card's VRAM, that
  card's temperature. Compact mode has one row, so there the aggregate reports
  the hottest card (`hot 60°C 😎`) and a selected card reports its own
  (`temp 57°C 😎`). Too short for the strip and the title still says
  `(2 cards · …)`.
- **GPU sensors: temperature, power and fan.** Each card reports its current
  temperature, its own slowdown threshold, the VRAM threshold (a limit, not a live
  memory reading — NVML exposes no live one), power draw against the enforced cap,
  and fan speed. The temperature glyph is graded against *that card's* slowdown
  limit, not the CPU thresholds: a 3090 sitting at 71°C of a 93°C limit is cool,
  and the old shared thresholds would have called it a warning. Sensors a card does
  not report stay blank — a stopped or passive fan is an NVML error, not 0%.

### Agent

- `get_metrics` responses carry `mem_free` and `mem_available` alongside
  `mem_used`; that pair is what makes the cache segment possible. Additive in
  both directions: old clients ignore the new fields, new clients default them
  to 0 and draw without the segment. On Windows `mem_free` equals
  `mem_available` (there is no separate figure).
- **`get_metrics` now enumerates all GPUs and reads their sensors.** The NVML path
  is preferred over `gfxinfo` (`libnvidia-ml.so.1` on Linux, `nvml.dll` on Windows):
  `gfxinfo` 0.1.2 only exposes the active device, and its NVIDIA fallback dlopens an
  unversioned soname that fails on Debian. So cards are enumerated by index and each carries `temp_c`, `temp_slowdown_c`,
  `temp_mem_max_c`, `power_w`, `power_limit_w`, `fan_pct`, `nvml_index` and `uuid`.
  Additive in both directions, in the way that keeps old payloads byte-identical:
  the primary card stays in `gpus` with its identity unchanged, the other cards go
  in a new `gpus_extra` that is absent — not empty — when there is only one, and an
  old client never sees the new key. A card with no sensors serializes exactly as
  an older agent's would. Per-card NVML failures degrade to missing fields rather
  than dropping the card.
- **`scripts/gpu_accounting_check.sh`** diffs the agent's per-card numbers against
  `nvidia-smi -q -x` on the box under test: card count, NVML-vs-nvidia-smi
  enumeration order (matched on UUID, `nvml_index` as the fallback), utilisation,
  VRAM, temperature and thresholds, power draw and cap, fan. It explains the one
  expected difference — the agent reads NVML v1 `used`, which counts the driver's
  reserved carve-out that `nvidia-smi` reports separately — and prints the
  aggregate the client would draw so the max-vs-average reading can be eyeballed
  under a real layer split.

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

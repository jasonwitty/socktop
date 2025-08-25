# socktop_agent (server)

Lightweight on‑demand metrics WebSocket server for the socktop TUI.

Highlights:
- Collects system metrics only when requested (keeps idle CPU <1%)
- Optional TLS (self‑signed cert auto‑generated & pinned by client)
- JSON for fast metrics / disks; protobuf (optionally gzipped) for processes
- Accurate per‑process CPU% on Linux via /proc jiffies delta
- Optional GPU & temperature metrics (disable via env vars)
- Simple token auth (?token=...) support

Run (no TLS):
```
cargo install socktop_agent
socktop_agent --port 3000
```
Enable TLS:
```
SOCKTOP_ENABLE_SSL=1 socktop_agent --port 8443
# cert/key stored under $XDG_DATA_HOME/socktop_agent/tls
```
Environment toggles:
- SOCKTOP_AGENT_GPU=0      (disable GPU collection)
- SOCKTOP_AGENT_TEMP=0     (disable temperature)
- SOCKTOP_TOKEN=secret     (require token param from client)
- SOCKTOP_AGENT_METRICS_TTL_MS=250 (cache fast metrics window)
- SOCKTOP_AGENT_PROCESSES_TTL_MS=1000
- SOCKTOP_AGENT_DISKS_TTL_MS=1000

Systemd unit example & full docs:
https://github.com/jasonwitty/socktop

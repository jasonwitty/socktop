# btop-remote (Rust)

A remote `btop`-style terminal UI to monitor system metrics over WebSockets, written in Rust.

## 📦 Build

```bash
cargo build --release
```

---

## 🚀 Run the Agent (on the remote host)

The agent collects system metrics and exposes them via WebSocket.

### 🔧 `sh` / `bash` example:

```sh
export AGENT_LISTEN=0.0.0.0:8765
export AGENT_TOKEN=mysharedsecret  # optional, for authentication

./target/release/remote-agent
```

### 🐟 `fish` shell example:

```fish
set -x AGENT_LISTEN 0.0.0.0:8765
set -x AGENT_TOKEN mysharedsecret  # optional

./target/release/remote-agent
```

---

## 🖥️ Run the TUI (on the local machine)

Connect to the remote agent over WebSocket:

```bash
./target/release/btop-remote ws://<REMOTE_IP>:8765/ws mysharedsecret
```

- Replace `<REMOTE_IP>` with your remote agent's IP address.
- Press `q` to quit.

---

## 🔐 Authentication (optional)

If `AGENT_TOKEN` is set on the agent, the TUI **must** provide it as the second argument.
If no token is set, authentication is disabled.

---

## 🧪 Example

```bash
# On remote machine:
export AGENT_LISTEN=0.0.0.0:8765
export AGENT_TOKEN=secret123
./target/release/remote-agent

# On local machine:
./target/release/btop-remote ws://192.168.1.100:8765/ws secret123
```

---

## 🛠 Dependencies

- Rust (2021 edition or later)
- WebSocket-compatible network (agent port must be accessible remotely)

---

## 🧹 Cleanup Build Artifacts

```bash
cargo clean
```

---

MIT License.

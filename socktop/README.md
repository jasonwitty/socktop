# socktop (client)

Minimal TUI client for the socktop remote monitoring agent.

Features:
- Connects to a socktop_agent over WebSocket / secure WebSocket
- Displays CPU, memory, swap, disks, network, processes, (optional) GPU metrics
- Self‑signed TLS cert pinning via --tls-ca
- Profile management with saved intervals
- Low CPU usage (request-driven updates)

Quick start:
```
cargo install socktop
socktop ws://HOST:3000/ws
```
With TLS (copy agent cert first):
```
socktop --tls-ca cert.pem wss://HOST:8443/ws
```
Demo mode (spawns a local agent automatically on first run prompt):
```
socktop --demo
```
Full documentation, screenshots, and advanced usage:
https://github.com/jasonwitty/socktop

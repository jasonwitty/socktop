# socktop

_socktop_ is a remote system monitor with a rich TUI, talking to an ultra lightweight agent over WebSockets.

<img src="./docs/socktop_demo_1_60.apng" width="100%">

## Resources

| Resource | Location |
| -------- | -------- |
| Website and online demo (yes it's real) | [socktop.io](https://www.socktop.io) |
| Quick Start guide   | [https://socktop.io/assets/docs/installation/quick-start.html](https://socktop.io/assets/docs/installation/quick-start.html)  |
| Prereqs | [https://socktop.io/assets/docs/installation/prerequisites.html](https://socktop.io/assets/docs/installation/prerequisites.html) |
| APT Install | [https://socktop.io/assets/docs/installation/apt.html](https://socktop.io/assets/docs/installation/apt.html) |
| Cargo Install | [https://socktop.io/assets/docs/installation/cargo.html](https://socktop.io/assets/docs/installation/cargo.html)
| Usage | [https://socktop.io/assets/docs/usage/general.html](https://socktop.io/assets/docs/usage/general.html)
| Auth Setup | [https://socktop.io/assets/docs/security/token.html](https://socktop.io/assets/docs/security/token.html) |
| TLS Setup | [https://socktop.io/assets/docs/security/tls.html](https://socktop.io/assets/docs/security/tls.html) |
| Monitoring Multiple Hosts | [tmux](https://socktop.io/assets/docs/advanced/tmux.html) / [zellij](https://socktop.io/assets/docs/advanced/zellij.html) |

---

## Platform Support

Linux (all flavors), ARM/Raspberry Pi (32b/64b), MacOS, Windows, RISC-V (experimental)

---

## Contributing

Contributions are welcome and you have the freedom to use whatever development tools you would like, as long as there is a human in the loop and all the clippy and unit tests pass you are good to submit a PR. Defects / Bugs just go ahead and fix and file a PR. New features, please create a issue in advance and let me know you are offering to build it. I don't want to be in a position where you worked for a couple of weeks on something and I don't want to merge it.

### Development

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo run -p socktop -- ws://127.0.0.1:3000/ws
# TLS (dev): first run will create certs under ~/.config/socktop_agent/tls/
cargo run -p socktop_agent -- --enableSSL --port 8443
```

### Auto-format on commit

A sample pre-commit hook that runs `cargo fmt --all` is provided in `.githooks/pre-commit`.
Enable it (one-time):

---

## License

MIT — see [LICENSE](LICENSE).

## Acknowledgements

- ratatui for the TUI
- sysinfo for system metrics
- tokio-tungstenite for WebSockets

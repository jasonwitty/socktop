//! Detection of whether the connected agent is running on this same machine.
//!
//! Process-kill is only offered for *local* agents. The reasoning is a
//! security one: the PIDs shown in the UI are reported by the agent, and when
//! the user asks to kill one, socktop sends the signal with its OWN local OS
//! privileges (a direct syscall — never over the network; see [`crate::proc_kill`]).
//! A PID is therefore only meaningful — and only safe to act on — when the
//! agent lives on this machine. If we acted on a remote agent's PIDs we would
//! be signalling whatever unrelated *local* process happened to share that
//! number.
//!
//! An address is considered local when it is loopback, or when we can bind an
//! ephemeral socket to it: a bind only succeeds for an address assigned to one
//! of this host's own network interfaces, so it also covers the case of an
//! agent reached over this machine's LAN IP. Detection fails closed — any
//! parse/resolution failure, or any resolved address that is not local,
//! disables the feature.

use std::net::{IpAddr, ToSocketAddrs, UdpSocket};

/// Returns true only if the agent reached at `ws_url` is on this machine.
pub fn agent_is_local(ws_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(ws_url) else {
        return false;
    };
    match parsed.host() {
        // IP literals can be checked directly without any name resolution.
        Some(url::Host::Ipv4(ip)) => ip_is_local(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => ip_is_local(IpAddr::V6(ip)),
        // A hostname (e.g. "localhost", or a LAN name) must resolve, and every
        // address it resolves to must be local. ws=80, wss=443 are the known
        // default ports; an explicit port in the URL is honored.
        Some(url::Host::Domain(domain)) => {
            let port = parsed.port_or_known_default().unwrap_or(0);
            match (domain, port).to_socket_addrs() {
                Ok(addrs) => {
                    let mut saw_any = false;
                    for addr in addrs {
                        saw_any = true;
                        if !ip_is_local(addr.ip()) {
                            return false;
                        }
                    }
                    saw_any
                }
                Err(_) => false,
            }
        }
        None => false,
    }
}

/// An address is local if it is loopback, or if we can bind an ephemeral
/// socket to it (only possible for an address on one of our own interfaces).
/// Port 0 requests an ephemeral port and sends no traffic.
fn ip_is_local(ip: IpAddr) -> bool {
    ip.is_loopback() || UdpSocket::bind((ip, 0)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::agent_is_local;

    #[test]
    fn loopback_hosts_are_local() {
        assert!(agent_is_local("ws://127.0.0.1:3000/ws"));
        assert!(agent_is_local("ws://localhost:3000/ws"));
        assert!(agent_is_local("ws://[::1]:3000/ws"));
        assert!(agent_is_local("wss://127.0.0.1/ws"));
    }

    #[test]
    fn public_addresses_are_not_local() {
        // 8.8.8.8 is not assigned to any local interface.
        assert!(!agent_is_local("ws://8.8.8.8:3000/ws"));
        // Documentation-range address, guaranteed not bound locally.
        assert!(!agent_is_local("ws://203.0.113.1:3000/ws"));
    }

    #[test]
    fn garbage_fails_closed() {
        assert!(!agent_is_local("not a url"));
        assert!(!agent_is_local(""));
    }
}

#!/usr/bin/env python3
"""Measure the socktop agent's real cost: CPU, RSS, threads, syscalls and payload sizes.

Pure standard library (no `websockets`, no `pip`), so it runs unchanged on a stock
Raspberry Pi OS where the agent actually has to be light.

Usage:
  perf_probe.py cadence  AGENT_PID ws://HOST:PORT/ws SECONDS
  perf_probe.py flood    AGENT_PID ws://HOST:PORT/ws SECONDS [CLIENTS]
  perf_probe.py detail   AGENT_PID ws://HOST:PORT/ws [PID]
  perf_probe.py tls      AGENT_PID wss://HOST:PORT/ws [CONNECTIONS]

tls    - n sequential TLS handshakes + WS upgrades + one get_metrics each. Set
         SOCKTOP_PERF_CA=/path/to/agent/cert.pem to verify the certificate the way
         `--tls-ca` does in the TUI; unset means "do not verify".
cadence  - drives the TUI's default polling (metrics 500 ms, processes 2 s, disks 5 s)
flood    - CLIENTS connections looping get_processes as fast as they can
detail   - one cold `get_process_metrics:PID`, then one inside the 250 ms TTL

Counters come from the agent's /proc entries: utime+stime ticks, VmRSS/VmHWM,
Threads and syscr (syscalls that read), so the syscall numbers transfer between
machines even when CPU percentages do not. Payload sizes are the raw wire bytes
(larger payloads are gzipped by the agent, so a small number can mean "compressed").
"""

import base64
import os
import socket
import ssl
import struct
import sys
import threading
import time


class Ws:
    """Minimal WebSocket client: text in, text-or-binary out, one frame per reply."""

    def __init__(self, url):
        secure = url.startswith("wss://")
        assert secure or url.startswith("ws://"), "expected a ws:// or wss:// URL"
        rest = url[len("wss://" if secure else "ws://"):]
        hostport, _, path = rest.partition("/")
        host, _, port = hostport.partition(":")
        self.sock = socket.create_connection((host, int(port or (443 if secure else 80))), timeout=30)
        if secure:
            ca = os.environ.get("SOCKTOP_PERF_CA")
            ctx = (
                ssl.create_default_context(cafile=ca)
                if ca
                else ssl._create_unverified_context()
            )
            self.sock = ctx.wrap_socket(self.sock, server_hostname=host)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (
            f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("handshake failed")
            buf += chunk
        if b"101" not in buf.split(b"\r\n", 1)[0]:
            raise RuntimeError(f"handshake rejected: {buf.splitlines()[0]!r}")
        self.buf = buf.split(b"\r\n\r\n", 1)[1]

    def send(self, text):
        payload = text.encode()
        mask = os.urandom(4)
        n = len(payload)
        if n < 126:
            header = struct.pack("!BB", 0x81, 0x80 | n)
        elif n < 65536:
            header = struct.pack("!BBH", 0x81, 0x80 | 126, n)
        else:
            header = struct.pack("!BBQ", 0x81, 0x80 | 127, n)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(header + mask + masked)

    def _recv_exact(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("connection closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def recv(self):
        b0, b1 = self._recv_exact(2)
        opcode = b0 & 0x0F
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack("!H", self._recv_exact(2))[0]
        elif n == 127:
            n = struct.unpack("!Q", self._recv_exact(8))[0]
        data = self._recv_exact(n)
        if opcode == 0x8:
            raise RuntimeError("server closed")
        return data


def read_io(pid):
    """Syscalls that read, or None if /proc/<pid>/io is not readable (the agent
    runs as another user, or a hardened kernel). Callers then print n/a."""
    try:
        for line in open(f"/proc/{pid}/io"):
            if line.startswith("syscr:"):
                return int(line.split(":")[1])
    except OSError:
        return None
    return 0


def delta(a, b):
    return None if a is None or b is None else b - a


def fmt(x, spec="{:.1f}"):
    return "n/a" if x is None else spec.format(x)


def cpu_ticks(pid):
    f = open(f"/proc/{pid}/stat").read()
    rest = f[f.rfind(")") + 2:].split()
    return int(rest[11]) + int(rest[12])


def status(pid, key):
    for line in open(f"/proc/{pid}/status"):
        if line.startswith(key + ":"):
            return line.split(":")[1].strip()
    return "?"


def summary(pid, t0, c0):
    ms = (cpu_ticks(pid) - c0) * 1000 / os.sysconf("SC_CLK_TCK")
    secs = time.monotonic() - t0
    print(f"  window {secs:5.1f}s   agent CPU {ms:8.0f} ms = {ms / secs / 10:.2f}% of one core")
    print(f"  VmRSS {status(pid, 'VmRSS')}   VmHWM {status(pid, 'VmHWM')}   Threads {status(pid, 'Threads')}")


def cadence(pid, url, secs):
    names = ["get_metrics", "get_processes", "get_disks"]
    sizes = {n: [] for n in names}
    reads = {n: [] for n in names}
    ws = Ws(url)

    def req(name):
        before = read_io(pid)
        ws.send(name)
        msg = ws.recv()
        d = delta(before, read_io(pid))
        if d is not None:
            reads[name].append(d)
        sizes[name].append(len(msg if isinstance(msg, bytes) else msg.encode()))

    c0, t0 = cpu_ticks(pid), time.monotonic()
    last_p = last_d = -1e9
    while time.monotonic() - t0 < secs:
        now = time.monotonic() - t0
        req("get_metrics")
        if now - last_p >= 2.0:
            req("get_processes")
            last_p = now
        if now - last_d >= 5.0:
            req("get_disks")
            last_d = now
        time.sleep(0.5)
    ws.sock.close()
    print(f"[cadence] one client, TUI default polling, {secs:.0f} s")
    summary(pid, t0, c0)
    for n in names:
        s = sizes[n]
        r = reads[n] or [0]
        have_io = bool(reads[n])
        print(
            f"  {n:14s} n={len(s):3d}  bytes avg {sum(s) / len(s):7.0f}   "
            f"syscr/req avg {fmt(sum(r) / len(r)) if have_io else 'n/a'} "
            f"max {max(r) if have_io else 'n/a'}"
        )


def flood(pid, url, secs, clients=10):
    stop = time.monotonic() + secs
    done = [0] * clients
    lock = threading.Lock()

    def worker(i):
        ws = Ws(url)
        n = 0
        while time.monotonic() < stop:
            ws.send("get_processes")
            ws.recv()
            n += 1
        ws.sock.close()
        done[i] = n

    c0, io0, t0 = cpu_ticks(pid), read_io(pid), time.monotonic()
    threads = [threading.Thread(target=worker, args=(i,)) for i in range(clients)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    total = sum(done)
    secs = time.monotonic() - t0
    ms = (cpu_ticks(pid) - c0) * 1000 / os.sysconf("SC_CLK_TCK")
    io = delta(io0, read_io(pid))
    print(f"[flood] {clients} clients hammering get_processes, {secs:.1f} s")
    print(f"  replies {total}   agent CPU {ms:.0f} ms = {ms / secs / 10:.0f}% of one core")
    per_io = "n/a" if io is None else f"{io / max(total, 1):.1f}"
    print(f"  per reply: {ms * 1000 / max(total, 1):.1f} us CPU   {per_io} read syscalls")
    print(f"  VmRSS {status(pid, 'VmRSS')}   Threads {status(pid, 'Threads')}")


def detail(pid, url, target="1"):
    ws = Ws(url)
    out = []
    for label in ("cold", "within TTL"):
        before_io = read_io(pid)
        c0 = cpu_ticks(pid)
        t0 = time.monotonic()
        ws.send(f"get_process_metrics:{target}")
        msg = ws.recv()
        wall = (time.monotonic() - t0) * 1000
        ms = (cpu_ticks(pid) - c0) * 1000 / os.sysconf("SC_CLK_TCK")
        out.append((label, wall, ms, delta(before_io, read_io(pid)), len(msg if isinstance(msg, bytes) else msg.encode())))
    # journal path, cold and warm (1 s TTL)
    journal = []
    for label in ("cold", "within TTL"):
        before_io = read_io(pid)
        c0 = cpu_ticks(pid)
        t0 = time.monotonic()
        ws.send(f"get_journal_entries:{target}")
        msg = ws.recv()
        wall = (time.monotonic() - t0) * 1000
        ms = (cpu_ticks(pid) - c0) * 1000 / os.sysconf("SC_CLK_TCK")
        journal.append((label, wall, ms, delta(before_io, read_io(pid)), len(msg if isinstance(msg, bytes) else msg.encode())))
    ws.sock.close()
    print(f"[detail] PID {target} on the agent host")
    for label, wall, ms, io, n in out:
        print(f"  get_process_metrics  {label:11s} wall {wall:7.1f} ms   agent CPU {ms:6.1f} ms   syscr {fmt(io, '{:d}'):>6s}   bytes {n}")
    for label, wall, ms, io, n in journal:
        print(f"  get_journal_entries  {label:11s} wall {wall:7.1f} ms   agent CPU {ms:6.1f} ms   syscr {fmt(io, '{:d}'):>6s}   bytes {n}")


def tls(pid, url, n=10):
    n = int(n)
    wall = 0.0
    for _ in range(n):
        t = time.perf_counter()
        w = Ws(url)
        w.send("get_metrics")
        w.recv()
        w.sock.close()
        wall += time.perf_counter() - t
    print(f"[tls] {n} connections (TLS handshake + upgrade + get_metrics): "
          f"{wall * 1000 / n:.1f} ms each, {n / wall:.1f} handshakes/s")


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    mode, pid, url = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    if mode == "cadence":
        cadence(pid, url, float(sys.argv[4]) if len(sys.argv) > 4 else 40)
    elif mode == "flood":
        flood(pid, url, float(sys.argv[4]) if len(sys.argv) > 4 else 10,
              int(sys.argv[5]) if len(sys.argv) > 5 else 10)
    elif mode == "detail":
        detail(pid, url, sys.argv[4] if len(sys.argv) > 4 else "1")
    elif mode == "tls":
        tls(pid, url, sys.argv[4] if len(sys.argv) > 4 else 10)
    else:
        print(f"unknown mode {mode}")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

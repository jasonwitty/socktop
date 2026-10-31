#!/usr/bin/env bash
# Verify how socktop's Memory gauge accounts for RAM — and why a process can
# show gigabytes of RSS while the bar barely moves.
#
#   used (what socktop shows) = MemTotal - MemAvailable
#
# MemAvailable counts *reclaimable* memory, and clean file-backed pages are
# reclaimable. So a process whose RSS is mostly a private file mapping
# (llama.cpp mmaps its GGUF model by default; so does any mmap-based loader)
# inflates its row in the process list but NOT the bar. Anonymous memory
# (heap, stacks, llama.cpp with --no-mmap) does move the bar.
#
# Phase A allocates 8 GiB of anonymous memory. Phase B maps a big file and
# faults it in. The bar's `used` segment moves ~8 GiB for A and sits still for
# B; in phase B the `cache` segment grows by the same amount, which is the
# point of splitting the bar. The probe prints the segment split as the client
# computes it from the agent's reply, next to the kernel's own figures.
#
# Needs python3 with the `websockets` package (pip install websockets) and a
# built agent: cargo build --release -p socktop_agent
#
#   scripts/mem_accounting_check.sh [path/to/bigfile]
#
set -uo pipefail

BIGFILE=${1:-}
PORT=${PORT:-3456}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
AGENT=${AGENT:-$ROOT/target/release/socktop_agent}
TMP=$(mktemp -d)
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

if [[ ! -x $AGENT ]]; then
  echo "no agent binary at $AGENT — run: cargo build --release -p socktop_agent" >&2
  exit 1
fi
if [[ -z $BIGFILE ]]; then
  BIGFILE=$(find "$HOME" -size +2G -type f 2>/dev/null | head -1)
fi

cat > "$TMP/probe.py" <<'PY'
import asyncio, gzip, json, sys
port, secs = int(sys.argv[1]), int(sys.argv[2])

def meminfo():
    d = {}
    for line in open('/proc/meminfo'):
        k, v = line.split(':', 1)
        d[k] = int(v.split()[0]) * 1024
    return d

async def main():
    import websockets
    G = 1024 ** 3
    async with websockets.connect(f'ws://127.0.0.1:{port}/ws') as ws:
        end = asyncio.get_event_loop().time() + secs
        while asyncio.get_event_loop().time() < end:
            await ws.send('get_metrics')
            raw = await ws.recv()
            if isinstance(raw, bytes):
                raw = gzip.decompress(raw)
            m = json.loads(raw)
            mi = meminfo()
            # What the gauge draws, straight off the wire: the agent sends
            # used / free / available and the client derives the cache slice.
            used = m['mem_used']
            free = m.get('mem_free', 0)
            cache = m.get('mem_available', 0) - free
            print(f"  gauge used {used/G:6.2f}G ({100*used/m['mem_total']:4.1f}%)"
                  f" + cache {cache/G:6.2f}G + free {free/G:5.2f}G"
                  f" = {(used+cache+free)/G:6.2f}G of {m['mem_total']/G:6.2f}G"
                  f" | kernel used {(mi['MemTotal']-mi['MemAvailable'])/G:6.2f}G"
                  f" free {mi['MemFree']/G:5.2f}G"
                  f" cache {mi['Cached']/G:6.2f}G"
                  f" avail {mi['MemAvailable']/G:5.2f}G", flush=True)
            await asyncio.sleep(1)

asyncio.run(main())
PY

cat > "$TMP/workload.py" <<'PY'
import mmap, os, sys, threading, time
phase, path = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else '')

def rss():
    for l in open(f'/proc/{os.getpid()}/status'):
        if l.startswith('VmRSS:'):
            return int(l.split()[1]) / 1024 ** 2  # GiB
    return 0.0

def report():
    while True:
        print(f'    workload RSS {rss():6.2f}G', flush=True)
        time.sleep(5)

threading.Thread(target=report, daemon=True).start()

if phase == 'anon':
    n = 8 * 1024 ** 3
    b = bytearray(n)
    for i in range(0, n, 4096):
        b[i] = 1
    print('    8 GiB of anonymous memory resident', flush=True)
else:
    fd = os.open(path, os.O_RDONLY)
    size = os.fstat(fd).st_size
    # Drop any page cache the file already has, otherwise phase B measures
    # nothing (the pages were resident before we started). No root needed.
    try:
        os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
    except OSError:
        pass
    mm = mmap.mmap(fd, 0, prot=mmap.PROT_READ, flags=mmap.MAP_PRIVATE)
    s = 0
    for i in range(0, size, 4096):
        s += mm[i]
    print(f'    {size/1024**3:.1f} GiB of file-backed pages resident (llama.cpp-style)', flush=True)
time.sleep(15)
if phase != 'anon':
    try:
        os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)  # tidy up after ourselves
    except OSError:
        pass
PY

"$AGENT" --port "$PORT" > "$TMP/agent.log" 2>&1 &
PIDS+=($!)
sleep 2

echo "== phase 0: baseline =="
python3 "$TMP/probe.py" "$PORT" 5

echo
echo "== phase A: 8 GiB anonymous heap — like llama.cpp --no-mmap, or any normal allocation =="
python3 "$TMP/probe.py" "$PORT" 25 > "$TMP/A.txt" &
PROBE=$!
sleep 1
python3 "$TMP/workload.py" anon > "$TMP/Awork.txt" &
PIDS+=($!)
wait "$PROBE"
cat "$TMP/Awork.txt"; cat "$TMP/A.txt"
while pgrep -f "$TMP/workload.py" >/dev/null; do sleep 1; done
sleep 5

if [[ -n $BIGFILE ]]; then
  echo
  echo "== phase B: $BIGFILE mapped read-only and faulted in — like llama.cpp with its defaults =="
  python3 "$TMP/probe.py" "$PORT" 90 > "$TMP/B.txt" &
  PROBE=$!
  sleep 1
  python3 "$TMP/workload.py" mmap "$BIGFILE" > "$TMP/Bwork.txt" &
  PIDS+=($!)
  wait "$PROBE"
  cat "$TMP/Bwork.txt"; cat "$TMP/B.txt"
else
  echo "(no file > 2 GiB found under \$HOME — skipping phase B, pass a path as \$1)"
fi

echo
echo "Expected: phase A moves the bar by ~8 GiB. In phase B the bar stays put while"
echo "the workload RSS (the same number socktop shows in the process list) climbs by"
echo "the file size and meminfo 'cache' grows to match — because those clean file"
echo "pages are counted as MemAvailable, not as used."

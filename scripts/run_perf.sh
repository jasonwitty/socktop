#!/usr/bin/env bash
# Reproducible agent performance run: startup latency, then the three probe modes.
#   ./run_perf.sh /path/to/socktop_agent LABEL [PORT]
set -u
BIN=${1:?usage: run_perf.sh /path/to/socktop_agent LABEL [PORT]}
LABEL=${2:?label required}
PORT=${3:-3777}
REPO=$(cd "$(dirname "$0")/.." && pwd)
PROBE="$REPO/scripts/perf_probe.py"

pkill -x socktop_agent 2>/dev/null || true
sleep 0.5

start=$(date +%s%N)
SOCKTOP_ENABLE_SSL=0 SOCKTOP_AGENT_GPU=0 SOCKTOP_AGENT_TEMP=1 "$BIN" -p "$PORT" >/tmp/perf_agent_$LABEL.log 2>&1 &
PID=$!
until python3 - "$PORT" <<'EOF'
import socket, sys
s = socket.socket()
try:
    s.connect(("127.0.0.1", int(sys.argv[1])))
except OSError:
    sys.exit(1)
sys.exit(0)
EOF
do sleep 0.05; done
end=$(date +%s%N)
echo "=== $LABEL ($($BIN --version 2>/dev/null || echo "$BIN")) ==="
echo "[startup] port accepting after $(( (end - start) / 1000000 )) ms   (pid $PID)"
sleep 1

python3 "$PROBE" cadence "$PID" "ws://127.0.0.1:$PORT/ws" 40
python3 "$PROBE" detail "$PID" "ws://127.0.0.1:$PORT/ws" 1
python3 "$PROBE" flood "$PID" "ws://127.0.0.1:$PORT/ws" 10 10
python3 "$PROBE" detail "$PID" "ws://127.0.0.1:$PORT/ws" 1

kill "$PID" 2>/dev/null
wait "$PID" 2>/dev/null
echo

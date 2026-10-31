#!/usr/bin/env bash
# Check GPU numbers against nvidia-smi, card by card.
#
# Multi-card GPU support rests on three claims that can only be checked on a box
# with real cards in it:
#
#   1. NVML enumeration order matches nvidia-smi order, so "GPU 0" in the TUI is
#      GPU 0 in nvidia-smi and not the other card.
#   2. The wire split (primary card in `gpus`, the rest in `gpus_extra`)
#      reassembles into the full set in the client: nothing dropped, nothing
#      duplicated.
#   3. The aggregate the TUI draws — VRAM summed, utilisation MAX rather than
#      average, temperature MAX — is the honest reading of what the cards are
#      doing. A llama.cpp layer split leaves one card pegged and one nearly
#      idle; averaging reports a healthy middle that neither card is in.
#
# The probe prints both sides of every number side by side, plus the aggregate as
# the client computes it. Pass a load command as $1 to watch a real layer split:
#
#   scripts/gpu_accounting_check.sh 'llama-bench -m /models/13B-Q4_K_M-00.gguf /models/13B-Q4_K_M-01.gguf'
#
# Needs python3 with the `websockets` package (pip install websockets), nvidia-smi,
# and a built agent: cargo build --release -p socktop_agent
#
#   PORT=3459 SECS=8 scripts/gpu_accounting_check.sh [load command]
#
# The agent under test and nvidia-smi need not be on this machine. Point HOST at
# an agent that is already running (then nothing is started locally) and SMI_CMD
# at whatever produces `nvidia-smi -q -x` XML on the box that has the cards —
# usually over ssh. That matters when the cards are not visible where the agent
# would otherwise run, e.g. GPUs passed through to a VM on a Proxmox host:
#
#   ssh -f -N -L 3461:127.0.0.1:3461 root@192.168.1.67
#   HOST=127.0.0.1 PORT=3461 SMI_CMD='ssh root@192.168.1.67 nvidia-smi -q -x' \
#     scripts/gpu_accounting_check.sh
#
set -uo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
AGENT=${AGENT:-$ROOT/target/release/socktop_agent}
HOST=${HOST:-127.0.0.1}
PORT=${PORT:-3459}
SMI_CMD=${SMI_CMD:-nvidia-smi -q -x}
SECS=${SECS:-8}
# Sampling windows differ — the agent caches for a second, nvidia-smi queries
# live — so exact equality is not the test. Being in the same place is.
UTIL_SLACK=${UTIL_SLACK:-15} # percentage points
MEM_SLACK=${MEM_SLACK:-134217728} # 128 MiB
TEMP_SLACK=${TEMP_SLACK:-6} # degrees C
POWER_SLACK=${POWER_SLACK:-0.15} # fraction of the larger reading
LOAD=${1:-}
# An agent somewhere else is assumed to be up already; only a local agent gets
# started (and stopped) by this script.
LOCAL_AGENT=1
case $HOST in
  127.0.0.1 | localhost) ;;
  *) LOCAL_AGENT=0 ;;
esac
TMP=$(mktemp -d)
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

if (( LOCAL_AGENT )) && [[ ! -x $AGENT ]]; then
  echo "no agent binary at $AGENT — run: cargo build --release -p socktop_agent" >&2
  exit 1
fi
if [[ $SMI_CMD == nvidia-smi* ]] && ! command -v nvidia-smi >/dev/null 2>&1; then
  echo "nvidia-smi not found — this check compares against it (or point SMI_CMD at a box that has it)" >&2
  exit 1
fi

cat > "$TMP/probe.py" <<'PY'
import asyncio, gzip, json, subprocess, sys
import xml.etree.ElementTree as ET

port, secs = int(sys.argv[1]), int(sys.argv[2])
UTIL_SLACK, MEM_SLACK = int(sys.argv[3]), int(sys.argv[4])
TEMP_SLACK, POWER_SLACK = float(sys.argv[5]), float(sys.argv[6])
host, smi_cmd = sys.argv[7], sys.argv[8]

GIB = 1024 ** 3


def gib(x):
    return f"{x / GIB:.2f}G"


def smi():
    """The nvidia-smi view, keyed by index, from a single XML query."""
    # -d cannot be combined with -x, so take the full query and pick the fields.
    out = subprocess.run(
        smi_cmd,
        shell=True,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    cards = {}
    for gpu in ET.fromstring(out).findall("gpu"):
        idx = int(gpu.findtext("minor_number", gpu.findtext("id", "0")))
        sensors = gpu.find("sensor_information")
        if sensors is None:
            sensors = gpu  # older drivers put the readings straight under <gpu>

        def num(*paths):
            """First path that yields a number, searched under <sensor_information>
            and then at the top level (field placement has moved between driver
            versions)."""
            for base in (sensors, gpu):
                for path in paths:
                    raw = (base.findtext(path) or "").strip()
                    if raw and not raw.upper().startswith("N/A"):
                        return float(raw.split()[0])
            return None

        cards[idx] = {
            "name": (gpu.findtext("product_name") or "").strip(),
            "uuid": (gpu.findtext("uuid") or "").strip(),
            "util": num("utilization/gpu_util", "utilization_gpu"),
            "mem_used": num("fb_memory_usage/used", "fb_memory_usage/dedicated/memory_used"),
            "mem_total": num("fb_memory_usage/total", "fb_memory_usage/dedicated/memory_total"),
            # nvidia-smi subtracts this carve-out from `used`; NVML v1, which the
            # agent uses, counts it. Printing it is what makes the two reconcile.
            "mem_reserved": num("fb_memory_usage/reserved"),
            "temp": num("temperature/gpu_temp", "temperature/current"),
            # NVML TemperatureThreshold::Slowdown is this field, not the shutdown
            # threshold; on this card they are 95 and 98.
            "temp_slowdown": num(
                "temperature/gpu_temp_slow_threshold", "temperature/gpu_temp_tlimit"
            ),
            "temp_mem_max": num("temperature/gpu_temp_max_mem_threshold"),
            "power": num(
                "gpu_power_readings/average_power_draw",
                "power_readings/power_draw",
                "gpu_power_readings/instant_power_draw",
            ),
            "power_limit": num(
                "gpu_power_readings/current_power_limit",
                "power_readings/current_power_limit",
                "power_readings/enforced_power_limit",
            ),
            "fan": num("fan_speed"),
        }
    return cards


async def agent():
    """What the agent put on the wire, reassembled the way the client does."""
    import websockets

    async with websockets.connect(f"ws://{host}:{port}/ws") as ws:
        await ws.send("get_metrics")
        raw = await ws.recv()
        if isinstance(raw, bytes):
            raw = gzip.decompress(raw)
        m = json.loads(raw)
    # The client rule: `gpus` holds the primary card, `gpus_extra` the rest, and
    # the key is absent altogether on a single-card host.
    cards = list(m.get("gpus") or []) + list(m.get("gpus_extra") or [])
    return cards, sorted(m.keys())


def row(label, a, s, fmt, slack):
    bad = a is not None and s is not None and abs(a - s) > slack
    return (
        f"  {'!!' if bad else '  '} {label:<10} agent {fmt(a):>10}"
        f"   nvidia-smi {fmt(s):>10}"
    )


async def main():
    for _ in range(max(1, secs // 2)):
        try:
            cards, keys = await agent()
        except Exception as e:
            print(f"  agent query failed: {e}", flush=True)
            await asyncio.sleep(1)
            continue
        smis = smi()
        print(f"agent reports {len(cards)} card(s); nvidia-smi reports {len(smis)}")
        print(
            "wire: `gpus_extra` present"
            if "gpus_extra" in keys
            else "wire: no `gpus_extra` key (single-card payload)"
        )
        if len(cards) != len(smis):
            print(f"  !! card count differs: agent {len(cards)} vs nvidia-smi {len(smis)}")

        # Match on UUID first: it is the only identifier that survives a change of
        # enumeration order. Matching on `nvml_index` afterwards is claim 1.
        by_uuid = {c.get("uuid"): c for c in cards if c.get("uuid")}
        for idx in sorted(smis):
            s = smis[idx]
            a, tag = by_uuid.get(s["uuid"]), "uuid"
            if a is None:
                a = next((c for c in cards if c.get("nvml_index") == idx), None)
                tag = "nvml_index"
            if a is None:
                print(f"card {idx} {s['name']}: !! not in the agent payload at all")
                continue
            print(f"card {idx} {s['name']}  (matched by {tag})")
            # The agent counts NVML v1 `used`, which includes the driver's
            # reserved carve-out; widen the slack by exactly that, so a genuine
            # mismatch still trips the flag and the carve-out does not.
            reserved = (s["mem_reserved"] or 0) * 1024 ** 2
            pct = lambda v: "—" if v is None else f"{v:.0f}%"
            deg = lambda v: "—" if v is None else f"{v:.0f}C"
            watt = lambda v: "—" if v is None else f"{v:.0f}W"
            byte = lambda v: "—" if v is None else gib(v)
            print(row("util", a.get("utilization_gpu_pct"), s["util"], pct, UTIL_SLACK))
            print(
                row(
                    "mem used",
                    a.get("mem_used_bytes"),
                    None if s["mem_used"] is None else s["mem_used"] * 1024 ** 2,
                    byte,
                    MEM_SLACK + reserved,
                )
            )
            print(
                row(
                    "mem total",
                    a.get("mem_total_bytes"),
                    None if s["mem_total"] is None else s["mem_total"] * 1024 ** 2,
                    byte,
                    MEM_SLACK,
                )
            )
            print(row("temp", a.get("temp_c"), s["temp"], deg, TEMP_SLACK))
            print(row("slowdown", a.get("temp_slowdown_c"), s["temp_slowdown"], deg, TEMP_SLACK))
            print(row("vram max", a.get("temp_mem_max_c"), s["temp_mem_max"], deg, TEMP_SLACK))
            cap = max(a.get("power_w") or 0, s["power"] or 0)
            print(row("power", a.get("power_w"), s["power"], watt, max(2.0, POWER_SLACK * cap)))
            print(row("power cap", a.get("power_limit_w"), s["power_limit"], watt, UTIL_SLACK))
            print(row("fan", a.get("fan_pct"), s["fan"], pct, 10))
            # Reconcile the VRAM difference instead of flagging it: NVML v1 counts
            # the driver's reserved carve-out, nvidia-smi reports it separately.
            if s["mem_reserved"] and a.get("mem_used_bytes") and s["mem_used"] is not None:
                delta = a["mem_used_bytes"] - s["mem_used"] * 1024 ** 2
                verdict = "explained" if abs(delta - reserved) <= MEM_SLACK else "NOT explained"
                print(
                    f"     {'  ' if verdict == 'explained' else '!!'} vram difference "
                    f"{gib(delta)} vs nvidia-smi reserved {gib(reserved)} — {verdict} "
                    "(agent uses NVML v1, which counts the carve-out)"
                )
            if a.get("fan_pct") is None and s["fan"] is not None:
                print(
                    "     note: the agent omits the fan where NVML errors (stopped or "
                    "passive fan); nvidia-smi prints a number anyway"
                )
            if a.get("name") != s["name"]:
                print(f"     note: name differs: {a.get('name')!r} vs {s['name']!r}")

        # The aggregate tab, computed the way the client computes it.
        utils = [c["utilization_gpu_pct"] for c in cards if c.get("utilization_gpu_pct") is not None]
        used = sum(c.get("mem_used_bytes") or 0 for c in cards)
        total = sum(c.get("mem_total_bytes") or 0 for c in cards)
        temps = [c["temp_c"] for c in cards if c.get("temp_c") is not None]

        def per_card(i, c):
            u = c.get("utilization_gpu_pct")
            return f"{c.get('nvml_index', i)}:" + ("—" if u is None else f"{u:.0f}%")

        print(
            "client aggregate: util max "
            + (f"{max(utils):.0f}%" if utils else "—")
            + (f" (an average would say {sum(utils) / len(utils):.0f}%)" if utils else "")
            + f", vram {gib(used)}/{gib(total)}"
            + (f", temp max {max(temps):.0f}C" if temps else "")
        )
        print("per-card util as the TUI prints it: " + "  ".join(
            per_card(i, c) for i, c in enumerate(cards)
        ))
        print(flush=True)
        await asyncio.sleep(2)


asyncio.run(main())
PY

if (( LOCAL_AGENT )); then
  "$AGENT" --port "$PORT" > "$TMP/agent.log" 2>&1 &
  PIDS+=($!)
  sleep 2
fi

echo "== agent on $HOST:$PORT vs nvidia-smi ($SMI_CMD) =="
if [[ -n $LOAD ]]; then
  # With a remote agent, wrap the load in whatever runs it on the GPU box:
  #   scripts/gpu_accounting_check.sh "ssh root@192.168.1.67 'llama-bench -m a.gguf b.gguf'"
  echo "running load: $LOAD"
  bash -c "$LOAD" > "$TMP/load.log" 2>&1 &
  LOADPID=$!
  python3 "$TMP/probe.py" "$PORT" "$SECS" "$UTIL_SLACK" "$MEM_SLACK" "$TEMP_SLACK" "$POWER_SLACK" "$HOST" "$SMI_CMD"
  wait "$LOADPID" 2>/dev/null
  echo "-- load output (head) --"
  head -12 "$TMP/load.log"
else
  python3 "$TMP/probe.py" "$PORT" "$SECS" "$UTIL_SLACK" "$MEM_SLACK" "$TEMP_SLACK" "$POWER_SLACK" "$HOST" "$SMI_CMD"
fi

echo
echo 'What to expect: every line inside its slack, and a leading "!!" only where the'
echo 'slack is exceeded or a card is missing. Under a two-card llama.cpp load the util'
echo 'max should sit near 100 while an average would say half that — the average is the'
echo 'number that would lie, which is why the pane shows max and the per-card line.'

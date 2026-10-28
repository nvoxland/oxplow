#!/usr/bin/env bash
# Prove an INSTALLED oxplow works on a repo that is not oxplow's checkout.
#
#   scripts/installed-smoke.sh <Oxplow.app | folder holding oxplow and oxplow-daemon>
#
# CI runs it on the macOS bundle after `tauri build`; locally, point it at a
# `bun run build:dev` output (`target/debug`) or a built `.app`. It makes a
# small Python project, then:
#   1. `oxplow --version` runs;
#   2. `oxplow extension new page hello`, `check` and `test` are clean there
#      (the SDK's throwaway oxplow activates foundation and bundled from
#      the copies embedded in the binary);
#   3. `oxplow-daemon --project <sample> --init` starts, and its extension
#      listing has oxplow-foundation and oxplow-bundled enabled with no errors.
# Nothing here reads the oxplow checkout. Exit 0 when every step holds.
set -euo pipefail

target="${1:?usage: scripts/installed-smoke.sh <Oxplow.app | folder with oxplow and oxplow-daemon>}"
if [ -d "$target/Contents/MacOS" ]; then
  bin_dir="$target/Contents/MacOS"
else
  bin_dir="$target"
fi
oxplow="$bin_dir/oxplow"
daemon="$bin_dir/oxplow-daemon"
for b in "$oxplow" "$daemon"; do
  [ -x "$b" ] || { echo "installed-smoke: no executable $b" >&2; exit 2; }
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
home="$work/home"
sample="$work/sample"
mkdir -p "$home" "$sample"
# The installed app, not this machine's oxplow: its own home and no repo.
export HOME="$home" OXPLOW_HOME="$home/oxplow"
unset OXPLOW_PROJECT_DIR OXPLOW_SHELL OXPLOW_BIN

step() { echo "==> $*"; }

step "a Python sample repo"
cat > "$sample/app.py" <<'PY'
def add(a, b):
    return a + b
PY
cat > "$sample/test_app.py" <<'PY'
from app import add


def test_add():
    assert add(1, 2) == 3
PY
git -C "$sample" init -q
git -C "$sample" add -A
git -C "$sample" -c user.name=smoke -c user.email=smoke@example.test commit -q -m init

step "oxplow --version"
"$oxplow" --version | grep -E '^oxplow [0-9]'

step "oxplow extension new page hello / check / test"
"$oxplow" extension new page hello --root "$sample"
"$oxplow" extension check hello --root "$sample"
"$oxplow" extension test hello --root "$sample"

step "oxplow-daemon starts on the sample and lists its extensions"
python3 - "$daemon" "$sample" <<'PY'
import json, re, subprocess, sys, threading, urllib.request

daemon, sample = sys.argv[1], sys.argv[2]
token = "smoke-token"
proc = subprocess.Popen(
    [daemon, "--project", sample, "--init", "--bind", "127.0.0.1:0", "--token-stdin"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
)
lines = []
base = []
def read():
    for line in proc.stdout:
        lines.append(line)
        m = re.search(r"listening on (http://\S+)", line)
        if m and not base:
            base.append(m.group(1))
threading.Thread(target=read, daemon=True).start()
try:
    # Stdin stays open: it is the daemon's lifeline.
    proc.stdin.write(token + "\n")
    proc.stdin.flush()
    import time
    deadline = time.time() + 120
    while not base and time.time() < deadline and proc.poll() is None:
        time.sleep(0.1)
    if not base:
        sys.exit("the daemon didn't start:\n" + "".join(lines))
    req = urllib.request.Request(
        base[0] + "/ipc/list_extensions", data=b"{}", method="POST",
        headers={"Authorization": "Bearer " + token, "content-type": "application/json"},
    )
    reply = json.load(urllib.request.urlopen(req, timeout=60))
    if reply.get("status") != "ok":
        sys.exit("list_extensions: " + json.dumps(reply))
    by_name = {e["name"]: e for e in reply["data"]}
    for name in ("oxplow-foundation", "oxplow-bundled"):
        e = by_name.get(name)
        if e is None:
            sys.exit(f"{name} is not listed: {sorted(by_name)}")
        if not e.get("enabled") or e.get("errors"):
            sys.exit(f"{name} should be enabled with no errors: {json.dumps(e)[:600]}")
        print(f"   {name}: enabled, no errors")
finally:
    proc.terminate()
    try:
        proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        proc.kill()
PY
echo "installed-smoke: ok"

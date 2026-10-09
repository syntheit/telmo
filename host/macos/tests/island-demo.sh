#!/bin/sh
# Visual check of the rebuild island with fake data. Starts a second Telmo with its own socket and state
# directory, then cycles running -> ok -> running -> failed. Never touches the real rebuild.json.
# Usage: island-demo.sh /path/to/Telmo.app/Contents/MacOS/Telmo   (quit the real Telmo first: both would register the hotkeys)
set -eu
bin=${1:?path to the Telmo binary}
dir=$(mktemp -d)
export TELMO_STATE_DIR=$dir TELMO_SOCKET=$dir/host.sock
"$bin" &
host=$!
trap 'kill $host 2>/dev/null; rm -rf "$dir"' EXIT INT TERM

write() { # state built to_build generation
  now=$(date +%s)
  printf '{"state":"%s","pid":4242,"started":%s,"finished":null,"built":%s,"to_build":%s,"fetched":0,"to_fetch":0,"last_line":"","error":null,"generation":%s}\n' \
    "$1" "$((now - 20))" "$2" "$3" "$4" > "$dir/rebuild.json.tmp"
  mv "$dir/rebuild.json.tmp" "$dir/rebuild.json"
}

sleep 2
for n in 0 8 16 24 32 40; do write running "$n" 40 null; sleep 2; done
write ok 40 40 142; echo "ok: retracts after 10 s"; sleep 14
write running 5 40 null; sleep 3
sed 's/"pid":4242/"pid":4242/' "$dir/rebuild.json" | sed 's/"running"/"failed"/' > "$dir/rebuild.json.tmp"; mv "$dir/rebuild.json.tmp" "$dir/rebuild.json"
echo "failed: stays until clicked; Ctrl-C to stop"; sleep 600

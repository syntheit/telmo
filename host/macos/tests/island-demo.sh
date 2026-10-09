#!/bin/sh
# Visual check of the rebuild island with fake data. Starts a second Telmo with its own socket and state
# directory, then walks evaluating -> downloading -> building -> activating -> ok, and a run that fails.
# Never touches the real rebuild.json.
# Usage: island-demo.sh /path/to/Telmo.app/Contents/MacOS/Telmo   (quit the real Telmo first: both would register the hotkeys)
set -eu
bin=${1:?path to the Telmo binary}
dir=$(mktemp -d)
export TELMO_STATE_DIR=$dir TELMO_SOCKET=$dir/host.sock
"$bin" &
host=$!
trap 'kill $host 2>/dev/null; rm -rf "$dir"' EXIT INT TERM

write() { # state phase built to_build fetched to_fetch generation
  now=$(date +%s)
  printf '{"state":"%s","phase":"%s","pid":4242,"started":%s,"finished":null,"built":%s,"to_build":%s,"fetched":%s,"to_fetch":%s,"last_line":"","error":null,"generation":%s}\n' \
    "$1" "$2" "$((now - 20))" "$3" "$4" "$5" "$6" "$7" > "$dir/rebuild.json.tmp"
  mv "$dir/rebuild.json.tmp" "$dir/rebuild.json"
}

run() { # one rebuild, ending in the state given
  echo "evaluating: sweep"; write running evaluating 0 0 0 0 null; sleep 6
  for n in 0 2 4 6 8 10; do echo "downloading $n/10"; write running downloading 0 0 "$n" 10 null; sleep 2; done
  for n in 0 8 16 24 32 40; do echo "building $n/40"; write running building "$n" 40 10 10 null; sleep 2; done
  echo "activating"; write running activating 40 40 10 10 null; sleep 5
}

sleep 2
run
write ok activating 40 40 10 10 142; echo "ok: retracts after 10 s"; sleep 14
run
write failed building 24 40 10 10 null
echo "failed: stays until clicked; Ctrl-C to stop"; sleep 600

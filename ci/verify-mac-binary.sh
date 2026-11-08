#!/bin/bash
# verify/site, the Mac's cross-built musl binary as it is: its sha256, then every route test on it (SITE_BIN), then
# bin/site-check url on it serving this commit's site/
set -euo pipefail
bin=$PWD/verify/site
sha256sum "$bin"
file "$bin"
SITE_BIN=$bin timeout 130 cargo nextest run --locked -p dash-site --no-fail-fast
out=$RUNNER_TEMP/serve
mkdir -p "$out"
"$bin" serve "$PWD/site" "$out/data" 0 > "$out/addr" 2> "$out/log" &
pid=$!
for _ in $(seq 50); do grep -q '^site: ' "$out/addr" && break; sleep 0.2; done
at=$(sed -n 's/^site: //p' "$out/addr")
echo "serving site/ at $at"
bin/site-check url "http://$at"
kill "$pid"

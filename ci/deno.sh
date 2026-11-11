#!/bin/bash
# The mirror's deno (ci.yml's test job): dash-gate's rust_js_passes_mix_lints_deno is mix lint's JS half, so it runs the
# deno bin/ci/lint-tools.sh pins (check.yml's lint), checksummed the same; the_deno_copies_are_mix_lints reads this pin (task rust-gate-mirrors-repo-scans)
set -euo pipefail
d=$RUNNER_TEMP/deno
mkdir -p "$d"
cd "$d"
curl -fsSL --retry 3 -o deno.zip https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-unknown-linux-gnu.zip
echo "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490  deno.zip" | sha256sum -c -
unzip -qo deno.zip
rm -f deno.zip
echo "$d" >> "$GITHUB_PATH"

#!/bin/sh
# Draws every named screen state to shots/out/<name>.png, offscreen: no window appears, and it works while the Mac is
# locked (items 0.13, 0.14 and 1.13). With names, only those. One command.
#
#   sh scripts/shots.sh [name ...]
#
# The states are listed in crates/enkidu-app/tests/shots.rs. Look at each picture after a change to what it shows.

set -eu
cd "$(dirname "$0")/.."
mkdir -p target
log=target/shots.log
if SHOTS_ONLY="$*" cargo test --locked -q -p enkidu-app --test shots > "$log" 2>&1; then code=0; else code=$?; fi
grep -E "^shots:" "$log" || true
if [ "$code" -ne 0 ]; then
  grep -v "^shots:" "$log" | tail -n 20
fi
exit "$code"

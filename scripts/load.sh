#!/bin/sh
# The load test (item 3.16): two workspaces of a 40,000-file repo, 420 changed files in each, on the
# whole app without a window, in a release build. It makes 120,000 files in a temporary folder and removes them.
# Prints its numbers (the lines that start with LOAD) and fails when a bound is missed.
#
#   sh scripts/load.sh

set -eu
cd "$(dirname "$0")/.."
mkdir -p target
# the ⌘-click lines need a language server: the one the app carries, when it was fetched (scripts/fetch-ty.sh)
if [ -z "${ENKIDU_LOAD_TY:-}" ] && [ -x target/tools/ty/bin/ty ]; then
  ENKIDU_LOAD_TY="$PWD/target/tools/ty/bin/ty"
  export ENKIDU_LOAD_TY
fi
log=target/load.log
if cargo test --release --locked -p enkidu-app --test load -- --nocapture --test-threads=1 > "$log" 2>&1; then code=0; else code=$?; fi
grep -E "^LOAD" "$log" || true
if [ "$code" -ne 0 ]; then
  grep -v "^LOAD" "$log" | tail -n 30
fi
exit "$code"

#!/bin/sh
# Waits until the Mac is locked, then draws the window offscreen once and says whether it worked
# (item 0.14). It runs a fixed copy of the pictures program, so a rebuild meanwhile doesn't change it.
#
#   sh scripts/shots-when-locked.sh [hours to wait, 10 by default]

cd "$(dirname "$0")/.." || exit 1
bin=$(ls -t target/debug/deps/shots-* 2>/dev/null | grep -v '\.d$' | head -1)
[ -x "$bin" ] || { echo "No pictures program yet: run cargo test -p enkidu-app --test shots first."; exit 1; }
out="$PWD/measure/out/locked"
mkdir -p "$out"
cp "$bin" "$out/shots-program"

locked() {
  osascript -l JavaScript -e 'ObjC.import("CoreGraphics"); const d = ObjC.deepUnwrap(ObjC.castRefToObject($.CGSessionCopyCurrentDictionary())); "" + (d.CGSSessionScreenIsLocked ?? false)'
}

limit=$(( ${1:-10} * 3600 ))
waited=0
echo "Watching for the lock from $(date '+%H:%M:%S')."
while [ "$(locked)" != "true" ]; do
  if [ "$waited" -ge "$limit" ]; then
    echo "The Mac did not lock within ${1:-10} hours."
    exit 2
  fi
  perl -e 'select(undef, undef, undef, 30)'
  waited=$((waited + 30))
done
# Give the lock a moment to settle, then check again, so a passing lock screen doesn't count.
perl -e 'select(undef, undef, undef, 20)'
echo "Locked: $(locked) at $(date '+%H:%M:%S')."
SHOTS_DIR="$out" "$out/shots-program"
code=$?
echo "After the picture: locked $(locked) at $(date '+%H:%M:%S'); the program's exit code $code."
ls -la "$out"/*.png 2>/dev/null
exit $code

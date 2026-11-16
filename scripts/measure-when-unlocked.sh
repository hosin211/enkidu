#!/bin/sh
# Waits until the Mac is unlocked and has stayed unlocked for 3 minutes, then runs scripts/measure.sh with the given
# plans. A measurement never runs on a locked Mac.
#
#   sh scripts/measure-when-unlocked.sh <hours to wait> <plan> [plan ...]

cd "$(dirname "$0")/.." || exit 1
hours=${1:-12}
shift
locked() {
  osascript -l JavaScript -e 'ObjC.import("CoreGraphics"); const d = ObjC.deepUnwrap(ObjC.castRefToObject($.CGSessionCopyCurrentDictionary())); "" + (d.CGSSessionScreenIsLocked ?? false)'
}
pause() { perl -e "select(undef, undef, undef, $1)"; }
limit=$((hours * 3600))
waited=0
calm=0
echo "Waiting for the Mac to be unlocked for 3 minutes, from $(date '+%H:%M:%S')."
while [ "$calm" -lt 180 ]; do
  if [ "$waited" -ge "$limit" ]; then
    echo "The Mac was not unlocked within $hours hours."
    exit 2
  fi
  if [ "$(locked)" = "true" ]; then calm=0; else calm=$((calm + 30)); fi
  pause 30
  waited=$((waited + 30))
done
echo "Unlocked since about $(date -v-3M '+%H:%M:%S'); measuring from $(date '+%H:%M:%S')."
sh scripts/measure.sh "$@"
echo "Done at $(date '+%H:%M:%S')."

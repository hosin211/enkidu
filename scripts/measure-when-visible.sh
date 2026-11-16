#!/bin/sh
# Runs scripts/measure.sh with each plan until one run had its window in view. GPUI draws nothing in a window that
# others cover (or that is minimised, on another Space, or on a sleeping display), so a covered run says nothing about
# what drawing costs. The window still opens in the background, without the keyboard.
# Tries every 4 minutes, 10 times at most. The binary is copied first, so builds meanwhile don't change it.
#
#   sh scripts/measure-when-visible.sh <plan> [plan ...]

cd "$(dirname "$0")/.." || exit 1
pause() { perl -e "select(undef, undef, undef, $1)"; }
mkdir -p target/measure-bin
cp target/release/enkidu-app target/measure-bin/enkidu-app || exit 1
for name in "${@:-lamp}"; do
  tries=0
  while :; do
    tries=$((tries + 1))
    report=$(ENKIDU_BIN=target/measure-bin/enkidu-app sh scripts/measure.sh "$name" 2>&1)
    if echo "$report" | grep -q -E 'covered for part|not measured|locked during'; then
      echo "$(date '+%H:%M') $name, try $tries: $(echo "$report" | grep -E '^== |not measured' | head -n 1)"
      if [ "$tries" -ge 10 ]; then
        echo "$name: not measured in view after 10 tries."
        break
      fi
      pause 240
    else
      echo "$(date '+%H:%M') $name, try $tries, in view:"
      echo "$report"
      break
    fi
  done
done

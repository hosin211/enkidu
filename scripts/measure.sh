#!/bin/sh
# Measures the real app in a real window (items 0.17 and 0.18). Each plan opens a window in the
# background without the keyboard, and the app quits by itself within two minutes. Don't type in it.
#
#   sh scripts/measure.sh [plan ...]
#
# Plans: rest1 and rest5 (nothing moves; 1 or 5 login shells), lamp (one lamp turns), lamp-30fps (the same with
# GPUI's cap for a window that isn't in front, about 30 frames a second), lamp-90s (one lamp for 90 s, memory read
# every 10 s), flood (100 MB printed in one terminal while the one beside it is typed into). CPU (over 10 s) and
# memory are read from outside with top and footprint; the flood's frames and typing are timed inside the app.
# Results go to measure/out/<plan>.*

set -u
cd "$(dirname "$0")/.." || exit 1
bin=${ENKIDU_BIN:-target/release/enkidu-app}
out=measure/out
mkdir -p "$out"
[ -x "$bin" ] || { echo "No $bin: build it first (cargo build --release --locked)."; exit 1; }

locked() {
  osascript -l JavaScript -e 'ObjC.import("CoreGraphics"); const d = ObjC.deepUnwrap(ObjC.castRefToObject($.CGSessionCopyCurrentDictionary())); "" + (d.CGSSessionScreenIsLocked ?? false)'
}
pause() { perl -e "select(undef, undef, undef, $1)"; }
footprint_mb() { footprint -p "$1" 2>/dev/null | sed -n 's/.*Footprint: \([0-9.]* [KMG]B\).*/\1/p' | head -1; }

plans=${*:-rest1 rest5 lamp flood}
for name in $plans; do
  plan=$name; seconds=40; keep_cap=""; trend=""
  case $name in
    lamp-30fps) plan=lamp; keep_cap=1 ;;
    lamp-90s) plan=lamp; seconds=90; trend=1 ;;
  esac
  if [ "$(locked)" = "true" ]; then
    echo "$name: not measured, the Mac is locked."
    continue
  fi
  rm -f "$out/$name.json" "$out/$name.top" "$out/$name.footprint" "$out/$name.trend"
  ENKIDU_MEASURE=$plan ENKIDU_SECONDS=$seconds ENKIDU_KEEP_CAP=$keep_cap ENKIDU_MEASURE_OUT="$out/$name.json" \
    "$bin" > "$out/$name.log" 2>&1 &
  pid=$!
  # A watchdog: the app quits by itself, and is ended at two minutes if it doesn't.
  ( pause 120; kill "$pid" 2>/dev/null ) &
  dog=$!
  if [ "$plan" != flood ]; then
    pause 15
    top -l 2 -s 10 -pid "$pid" -stats pid,cpu,mem,threads 2>/dev/null | tail -1 > "$out/$name.top"
    footprint -p "$pid" > "$out/$name.footprint" 2>&1
    if [ -n "$trend" ]; then
      t=25
      while kill -0 "$pid" 2>/dev/null && [ $t -le 85 ]; do
        echo "${t}s $(footprint_mb "$pid")" >> "$out/$name.trend"
        pause 10
        t=$((t + 10))
      done
    fi
  fi
  wait "$pid"
  code=$?
  kill "$dog" 2>/dev/null
  state=""
  [ "$(locked)" = "true" ] && state=" (the Mac locked during this run: don't trust it)"
  # GPUI doesn't draw a window that others cover (or that is minimised, or on another Space) at all
  share=$(sed -n 's/.*"visible_share":\([0-9.]*\).*/\1/p' "$out/$name.json" 2>/dev/null)
  case $share in
    1.00 | 0.9*) ;;
    "") ;;
    *) state="$state (its window was covered for part of the run, visible $share of the time: run it again where it can be seen)" ;;
  esac
  echo "== $name: exit $code$state"
  cat "$out/$name.json" 2>/dev/null
  [ -s "$out/$name.top" ] && echo "top (pid cpu% mem threads): $(cat "$out/$name.top")"
  grep -i -E 'Footprint:' "$out/$name.footprint" 2>/dev/null | head -1
  [ -s "$out/$name.trend" ] && echo "memory every 10 s: $(tr '\n' ';' < "$out/$name.trend")"
done

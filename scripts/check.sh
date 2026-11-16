#!/bin/sh
# One command for the whole check (item 1.5): the format, the lints, every test, and the layers. Exit 0 only if all
# pass.
# Each step's output goes to target/check-<step>.log.
#
#   sh scripts/check.sh

set -u
cd "$(dirname "$0")/.." || exit 1

mkdir -p target
failed=""
step() {
  name=$1
  shift
  start=$(date +%s)
  if "$@" > "target/check-$name.log" 2>&1; then
    echo "ok    $name, $(($(date +%s) - start)) s"
  else
    echo "FAIL  $name, $(($(date +%s) - start)) s: see target/check-$name.log"
    failed="$failed $name"
  fi
}

# enkidu-core, enkidu-term, enkidu-model and enkidu-syntax never depend on GPUI.
layers() {
  for c in enkidu-core enkidu-term enkidu-model enkidu-syntax; do
    n=$(cargo tree --locked -p "$c" -e normal,build,dev --prefix none | grep -c -E '^gpui[-_a-z]* v')
    echo "$c: $n GPUI crates"
    [ "$n" -eq 0 ] || return 1
  done
}

step format cargo fmt --check
step lints cargo clippy --workspace --all-targets --locked -- -D warnings
# six test threads, like the six build jobs: the Mac stays usable while you work, and tests that wait on a
# fake gh or twg don't time out because ten others run beside them
RUST_TEST_THREADS=6
export RUST_TEST_THREADS
step tests cargo test --workspace --locked --no-fail-fast
# the timed guards of item 2.9, in a release build, alone
step guards cargo test --release --locked -p enkidu-term --test guards --test memory_guard -- --test-threads=1 --nocapture
# the timed guard of item 3.4: a 700-file diff, with the real text system and no window
step big-diff cargo test --release --locked -p enkidu-diff --test big_diff
step layers layers
# the maintainers' own last step, where their release script is (it is not part of this repository)
if [ -f scripts/public.sh ]; then
  step public sh scripts/public.sh check
fi

awk '/^test result:/ { p += $4; f += $6; i += $8 } END { printf "Tests: %d passed, %d failed, %d ignored.\n", p, f, i }' \
  target/check-tests.log
if [ -z "$failed" ]; then
  echo "All passed."
  exit 0
fi
echo "Failed:$failed"
exit 1

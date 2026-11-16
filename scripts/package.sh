#!/bin/sh
# Makes the file a release carries: dist/Enkidu-<version>-<arch>.zip, the app ready to be moved to Applications.
#
# The app is built in a folder of its own (target/package) with this Mac's folder names taken out of the program
# (a panic names the source file it came from, and those paths would otherwise start with your home folder). It is
# signed ad hoc, so no developer's name is in it, and it carries LICENSE, NOTICE and the notices of every package it
# is built from (scripts/notices.sh).
#
# A Mac that downloads it will not open it at first, because no Apple Developer ID signed it. README.md, "Install",
# says how to allow it once.
#
#   sh scripts/fetch-ty.sh    # first, if the app should carry the language server for Python
#   sh scripts/package.sh

set -eu
cd "$(dirname "$0")/.."
root=$(pwd -P)

version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "Refused: Cargo.toml names no version."; exit 1; }
arch=$(uname -m)

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/package}"
export CARGO_TARGET_DIR
# what stands in the program in place of this Mac's folders: "~" for the home folder, "target" for the build folder
RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$CARGO_TARGET_DIR=target"
export RUSTFLAGS

mkdir -p "$CARGO_TARGET_DIR" dist
app="$CARGO_TARGET_DIR/app/Enkidu.app"
rm -rf "$CARGO_TARGET_DIR/app"
mkdir -p "$CARGO_TARGET_DIR/app"

# built first (the notices are read from the packages cargo fetched for this build)
cargo build --release --locked -p enkidu-app -p enkidu-cli
sh scripts/notices.sh > "$CARGO_TARGET_DIR/THIRD-PARTY-NOTICES.txt"

ENKIDU_SIGN_IDENTITY=- ENKIDU_INSTALL_TARGET="$app" ENKIDU_NOTICES="$CARGO_TARGET_DIR/THIRD-PARTY-NOTICES.txt" \
  sh scripts/install.sh

# what is handed out names nobody and nothing of this Mac
codesign -dv "$app" 2>&1 | grep -q "^Signature=adhoc" || { echo "Refused: the app is not signed ad hoc."; exit 1; }
for program in enkidu-app enkidu; do
  if strings -a "$app/Contents/MacOS/$program" | grep -q -F "$HOME"; then
    echo "Refused: $program still carries $HOME."
    exit 1
  fi
done

zip="dist/Enkidu-$version-$arch.zip"
rm -f "$zip"
# (without the marks macOS keeps on this Mac's files: they are no part of the app, and a zip would carry them)
ditto -c -k --keepParent --norsrc --noextattr --noqtn --noacl "$app" "$zip"
if unzip -l "$zip" | grep -q "__MACOSX"; then
  echo "Refused: the zip carries more than the app."
  exit 1
fi
echo "$zip: $(du -h "$zip" | cut -f1 | tr -d ' '), sha256 $(shasum -a 256 "$zip" | cut -d' ' -f1)"

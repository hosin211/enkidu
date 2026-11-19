#!/bin/sh
# Builds the release app and installs it as "Enkidu.app" in /Applications, signed with the Mac's own developer
# certificate (ad hoc when there is none), with its own identity, its icon and the microphone line. It never
# installs over another app: a target that is there already and is not Enkidu is refused.
#
# The signature decides whether macOS remembers what you allowed (the microphone, for voice in a terminal). macOS
# keeps such an answer by the app's "designated requirement". Signed ad hoc, that is the hash of this very build, so
# after every install it asks again (its own log says "Failed to match existing code requirement"; item 17.2).
# Signed with Xcode's "Apple Development" certificate from your keychain, it is the app's identifier and the
# certificate's name: the same for every build. Nothing is sent anywhere (no timestamp server).
#   ENKIDU_SIGN_IDENTITY=<a name or 40 hex digits>  another identity      ENKIDU_SIGN_IDENTITY=-  ad hoc
#
# The icon is bundle/icon.icns: two wedges standing side by side, the second one in the app's own blue (DESIGN.md,
# "The mark"). Its drawing is bundle/icon.svg, and the sizes in the .icns were made from that drawing.
#
#   sh scripts/install.sh

set -eu
cd "$(dirname "$0")/.."

ID="io.github.hosin211.enkidu"
NAME="Enkidu"
# the version is the workspace's, so the app says the number its source has
VERSION=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
TARGET="${ENKIDU_INSTALL_TARGET:-/Applications/$NAME.app}"

refuse() { echo "Refused: $1"; exit 1; }
[ -n "$VERSION" ] || refuse "Cargo.toml names no version."
if [ -e "$TARGET" ]; then
  have=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$TARGET/Contents/Info.plist" 2>/dev/null || echo none)
  [ "$have" = "$ID" ] || refuse "$TARGET is there already and is not $NAME (its identity: $have)."
fi

# where cargo builds: target/ here, unless CARGO_TARGET_DIR names another folder (scripts/package.sh does)
built="${CARGO_TARGET_DIR:-target}"

cargo build --release --locked -p enkidu-app -p enkidu-cli

stage="$built/bundle/$NAME.app"
rm -rf "$stage"
mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
cp "$built/release/enkidu-app" "$stage/Contents/MacOS/enkidu-app"
# its own command, next to its program: the one its terminals and hooks run
cp "$built/release/enkidu" "$stage/Contents/MacOS/enkidu"
# the language server for Python (phase 12), when `sh scripts/fetch-ty.sh` has fetched it: beside the program, where
# the core looks for it. Without it ⌘-click in Python has nothing to ask
if [ -x target/tools/ty/bin/ty ]; then
  cp target/tools/ty/bin/ty "$stage/Contents/MacOS/ty"
  carried="ty $(target/tools/ty/bin/ty --version | awk '{print $2}')"
else
  carried="no ty (sh scripts/fetch-ty.sh fetches it)"
fi
[ -s bundle/icon.icns ] || refuse "bundle/icon.icns is missing: the app would have no icon."
cp bundle/icon.icns "$stage/Contents/Resources/icon.icns"
# the app carries its license and what it copied from others; and, where ENKIDU_NOTICES names a file, the notices of
# the packages it is built from (scripts/package.sh makes that file for a release)
cp LICENSE NOTICE "$stage/Contents/Resources/"
if [ -n "${ENKIDU_NOTICES:-}" ]; then
  [ -s "$ENKIDU_NOTICES" ] || refuse "$ENKIDU_NOTICES is missing or empty."
  cp "$ENKIDU_NOTICES" "$stage/Contents/Resources/THIRD-PARTY-NOTICES.txt"
fi
cat > "$stage/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleExecutable</key><string>enkidu-app</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Claude Code in Enkidu's terminals uses the microphone for voice dictation.</string>
</dict>
</plist>
PLIST
plutil -lint "$stage/Contents/Info.plist" >/dev/null
# voice: without this line macOS refuses the microphone to anything a terminal of the app runs (item 2.16)
/usr/libexec/PlistBuddy -c 'Print :NSMicrophoneUsageDescription' "$stage/Contents/Info.plist" >/dev/null \
  || refuse "the bundle has no microphone line."
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$stage/Contents/Info.plist" 2>/dev/null)" = "icon" ] \
  || refuse "the bundle doesn't name its icon."
sign="${ENKIDU_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null \
  | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) "Apple Development: .*/\1/p' | head -1)}"
# (what an earlier run's codesign said is not this run's)
rm -f "$built/bundle/codesign.err"
if [ -n "$sign" ] && [ "$sign" != "-" ] \
  && codesign --force --sign "$sign" --timestamp=none "$stage" 2>"$built/bundle/codesign.err"; then
  :
else
  # no certificate, or its key could not be used (a locked keychain): the app still installs, and says so below
  [ -s "$built/bundle/codesign.err" ] && sed 's/^/  codesign: /' "$built/bundle/codesign.err"
  codesign --force --sign - --timestamp=none "$stage"
fi
codesign --verify --strict "$stage"
case "$(codesign -d -r- "$stage" 2>&1 | sed -n 's/^designated => //p')" in
  *cdhash* | "" ) kept="NOT kept: signed ad hoc, so macOS will ask for the microphone again after this install" ;;
  * ) kept="kept from build to build: macOS remembers the microphone answer" ;;
esac

rm -rf "$TARGET.new"
cp -R "$stage" "$TARGET.new"
rm -rf "$TARGET"
mv "$TARGET.new" "$TARGET"
# (Finder and the Dock keep an app's icon by its date: a new date makes them look again)
touch "$TARGET"

echo "Installed $TARGET ($(du -sh "$TARGET" | cut -f1), with $carried, its program $(stat -f %z "$TARGET/Contents/MacOS/enkidu-app") bytes, its command $(stat -f %z "$TARGET/Contents/MacOS/enkidu") bytes)."
# its identity, and who signed it: "Signature=adhoc", or the certificate's name
codesign -dv --verbose=2 "$TARGET" 2>&1 | grep -E '^(Identifier=|Signature=adhoc|Authority=Apple Development)' | sed 's/^/  /'
echo "  What macOS knows the app by is $kept"
echo "  Icon=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$TARGET/Contents/Info.plist").icns, $(stat -f %z "$TARGET/Contents/Resources/icon.icns") bytes"

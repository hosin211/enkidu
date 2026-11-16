#!/bin/sh
# Fetches the language server for Python that the app carries (phase 12): Astral's `ty`, the version
# pinned here, into target/tools/ty (never onto your PATH). `scripts/install.sh` copies it beside the app's
# own program when it is there; without it ⌘-click uses the app's own rules alone.
#
# It downloads about 13 MB from PyPI with `uv`, once. Not on the phone's hotspot.
#
#   sh scripts/fetch-ty.sh

set -eu
cd "$(dirname "$0")/.."
VERSION="0.0.104"
tool="target/tools/ty"

if [ -x "$tool/bin/ty" ] && [ "$("$tool/bin/ty" --version 2>/dev/null | awk '{print $2}')" = "$VERSION" ]; then
  echo "ty $VERSION is there already: $tool/bin/ty"
  exit 0
fi
gateway=$(route -n get default 2>/dev/null | awk '/gateway:/ {print $2}')
if [ "$gateway" = "172.20.10.1" ] && [ "${ENKIDU_ANY_NETWORK:-}" != "1" ]; then
  echo "Refused: this Mac is on a phone's hotspot. Run it again on Wi-Fi, or with ENKIDU_ANY_NETWORK=1."
  exit 1
fi
command -v uv >/dev/null 2>&1 || { echo "Refused: uv is not on this Mac (https://docs.astral.sh/uv)."; exit 1; }
rm -rf "$tool"
mkdir -p target/tools
uv venv -q "$tool"
uv pip install -q --python "$tool/bin/python" "ty==$VERSION"
got=$("$tool/bin/ty" --version | awk '{print $2}')
[ "$got" = "$VERSION" ] || { echo "Refused: got ty $got, not $VERSION."; exit 1; }
file "$tool/bin/ty" | grep -q "Mach-O" || { echo "Refused: $tool/bin/ty is not a program."; exit 1; }
echo "ty $VERSION: $tool/bin/ty ($(du -h "$tool/bin/ty" | cut -f1))"

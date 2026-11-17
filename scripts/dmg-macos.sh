#!/bin/bash
# Packs dist/MacroFlow.app (from bundle-macos.sh) into dist/MacroFlow-macos-arm64.dmg,
# with an Applications shortcut so it can be dragged straight in.
#
#   ./scripts/bundle-macos.sh && ./scripts/dmg-macos.sh
set -euo pipefail
cd "$(dirname "$0")/.."

APP=dist/MacroFlow.app
DMG=dist/MacroFlow-macos-arm64.dmg
[ -d "$APP" ] || { echo "Run scripts/bundle-macos.sh first" >&2; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/MacroFlow.app"
ln -s /Applications "$STAGE/Applications"

rm -f "$DMG"
hdiutil create -quiet -volname MacroFlow -srcfolder "$STAGE" -format UDZO -ov "$DMG"
echo "Built $DMG"

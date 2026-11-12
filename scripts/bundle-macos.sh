#!/bin/bash
# Builds dist/MacroFlow.app: the release build, its helper programs and the icon,
# so MacroFlow can live in /Applications like any other app.
#
#   ./scripts/bundle-macos.sh
#
# The app is signed "ad hoc" (no Apple developer account needed). macOS asks for
# Accessibility and Screen Recording permission for MacroFlow itself, the first
# time it needs them.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
APP=dist/MacroFlow.app
BIN="$APP/Contents/MacOS"

cargo build --release

rm -rf "$APP"
mkdir -p "$BIN" "$APP/Contents/Resources"
cp target/release/macroflow "$BIN/MacroFlow"

# The Swift helpers are built by build.rs. MacroFlow looks for them next to
# itself, so copy the newest of each into the app.
for helper in macroflow-ocr macroflow-notify macroflow-marker macroflow-select; do
  found=$(find target/release/build -name "$helper" -type f -perm -u+x -print0 2>/dev/null | xargs -0 ls -t 2>/dev/null | head -n 1 || true)
  if [ -n "$found" ]; then
    cp "$found" "$BIN/"
  else
    echo "warning: $helper wasn't built, so the feature that uses it won't work in the app" >&2
  fi
done

cp assets/MacroFlow.icns "$APP/Contents/Resources/MacroFlow.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>MacroFlow</string>
  <key>CFBundleDisplayName</key><string>MacroFlow</string>
  <key>CFBundleIdentifier</key><string>app.macroflow.MacroFlow</string>
  <key>CFBundleExecutable</key><string>MacroFlow</string>
  <key>CFBundleIconFile</key><string>MacroFlow</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Sign the helpers first, then the app around them.
for f in "$BIN"/*; do codesign --force --sign - "$f"; done
codesign --force --sign - "$APP"

echo "Built $APP (version $VERSION)"

#!/bin/sh
set -eu
cd "$(dirname "$0")/.."

export MACOSX_DEPLOYMENT_TARGET=12.0
cargo build --release --locked

app_bundle="dist/Fomo.app"
mkdir -p "$app_bundle/Contents/MacOS" "$app_bundle/Contents/Resources"
cp target/release/fomo "$app_bundle/Contents/MacOS/fomo"
cp packaging/Info.plist "$app_bundle/Contents/Info.plist"
/usr/bin/codesign --force --sign - "$app_bundle"
printf '\nBuilt %s\nOpen it in Finder, or run: open "%s"\n' "$app_bundle" "$app_bundle"

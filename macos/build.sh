#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

if [[ -z "${DEVELOPER_DIR:-}" && -d /Applications/Xcode.app ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
configuration="${1:-release}"
xcrun swift build -c "$configuration"
bin_dir="$(xcrun swift build -c "$configuration" --show-bin-path)"
app="build/lorca.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin_dir/lorca-macos" "$app/Contents/MacOS/lorca"
cp Sources/LorcaMac/Resources/orca.txt "$app/Contents/Resources/orca.txt"
cp Info.plist "$app/Contents/Info.plist"
# Finder/iCloud can attach metadata to generated bundles that codesign rejects.
xattr -cr "$app"
codesign --force --sign - "$app"
echo "Built $PWD/$app"

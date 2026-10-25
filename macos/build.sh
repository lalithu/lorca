#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

if [[ -z "${DEVELOPER_DIR:-}" && -d /Applications/Xcode.app ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
# A minimal rustup install need not alter the user's shell profile.
if ! command -v cargo >/dev/null && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
# This CLT SDK needs a SwiftUI macro plugin absent from the CLT distribution.
# Full Xcode and explicit SDK selections keep their normal behavior.
if [[ -z "${SDKROOT:-}" && ! -d /Applications/Xcode.app && -d /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk ]]; then
    export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
fi
configuration="${1:-release}"
(cd .. && cargo build --release)
xcrun swift build -c "$configuration"
bin_dir="$(xcrun swift build -c "$configuration" --show-bin-path)"
app="build/lorca.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin_dir/lorca-macos" "$app/Contents/MacOS/lorca"
cp ../target/release/lorca "$app/Contents/MacOS/lorca-core"
cp Sources/LorcaMac/Resources/orca.txt "$app/Contents/Resources/orca.txt"
cp Info.plist "$app/Contents/Info.plist"
# Finder/iCloud can attach metadata to generated bundles that codesign rejects.
xattr -cr "$app"
codesign --force --sign - "$app"
echo "Built $PWD/$app"

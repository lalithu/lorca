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
# Replace Mach-O files with new inodes. Overwriting an executable in place can
# leave macOS's cached signature for the old binary and cause launch-time SIGKILL.
cp "$bin_dir/lorca-macos" "$app/Contents/MacOS/lorca.new"
mv -f "$app/Contents/MacOS/lorca.new" "$app/Contents/MacOS/lorca"
cp ../target/release/lorca "$app/Contents/MacOS/lorca-core.new"
mv -f "$app/Contents/MacOS/lorca-core.new" "$app/Contents/MacOS/lorca-core"
cp Sources/LorcaMac/Resources/orca.txt "$app/Contents/Resources/orca.txt"
rm -rf "$app/Contents/Resources/jobapps"
mkdir -p "$app/Contents/Resources/jobapps"
cp ../jobapps/*.py "$app/Contents/Resources/jobapps/"
cp -R ../jobapps/defaults ../jobapps/vendor "$app/Contents/Resources/jobapps/"
cp AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cp Info.plist "$app/Contents/Info.plist"
# Finder/iCloud can attach metadata to generated bundles that codesign rejects.
xattr -cr "$app"
codesign --force --sign - "$app/Contents/MacOS/lorca-core"
codesign --force --sign - "$app"
# Exercise the bundled core, not target/release: {} must reach input validation
# without loading a model, creating data, or running a research task.
python3 - "$app/Contents/MacOS/lorca-core" <<'PYCORE'
import pathlib, subprocess, sys
core = str(pathlib.Path(sys.argv[1]).resolve())
result = subprocess.run([core, 'job-session'], input=b'{}', capture_output=True, timeout=10)
if result.returncode != 1 or b'Missing model' not in result.stderr:
    raise SystemExit(f'Bundled research core failed startup (exit {result.returncode}): {result.stderr.decode(errors="replace")[:300]}')
PYCORE
# lorca rebuild installs with this same toolchain environment and target cache.
# Ordinary app-only builds keep their existing behavior.
if [[ "${LORCA_INSTALL_CLI:-0}" == "1" ]]; then
    (cd .. && cargo install --path . --locked --force --offline --target-dir target)
fi
touch "$app"
echo "Built $PWD/$app"

# Only a successful, signed build may replace running processes. Packaging and
# isolated acceptance tests can opt out explicitly without touching live clients.
if [[ "${LORCA_BUILD_ONLY:-0}" == "1" ]]; then
    echo "Build-only mode: running Lorca was not restarted."
else
    python3 restart.py "$PWD/$app"
fi

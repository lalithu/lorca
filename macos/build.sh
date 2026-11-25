#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

source ./toolchain.sh
# A minimal rustup install need not alter the user's shell profile.
if ! command -v cargo >/dev/null && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
app="build/lorca.app"
ui_only="${LORCA_UI_ONLY:-0}"
if [[ "$ui_only" == "1" ]]; then
    if [[ ! -x "$app/Contents/MacOS/lorca-core" ]]; then
        echo "No bundled Rust core found. Run lorca rebuild once before lorca rebuild ui." >&2
        exit 1
    fi
    configuration="${1:-debug}"
else
    configuration="${1:-release}"
    (cd .. && cargo build --release --locked)
fi
# One wrapper supplies the exact SDK and linked SDK version for builds and tests.
bash ./swift.sh build -c "$configuration"
bin_dir="$(bash ./swift.sh build -c "$configuration" --show-bin-path)"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# Replace Mach-O files with new inodes. Overwriting an executable in place can
# leave macOS's cached signature for the old binary and cause launch-time SIGKILL.
cp "$bin_dir/lorca-macos" "$app/Contents/MacOS/lorca.new"
mv -f "$app/Contents/MacOS/lorca.new" "$app/Contents/MacOS/lorca"
if [[ "$ui_only" != "1" ]]; then
    cp ../target/release/lorca "$app/Contents/MacOS/lorca-core.new"
    mv -f "$app/Contents/MacOS/lorca-core.new" "$app/Contents/MacOS/lorca-core"
    cp Sources/LorcaMac/Resources/orca.txt "$app/Contents/Resources/orca.txt"
    rm -rf "$app/Contents/Resources/jobapps"
    mkdir -p "$app/Contents/Resources/jobapps"
    cp ../jobapps/*.py "$app/Contents/Resources/jobapps/"
    cp -R ../jobapps/defaults ../jobapps/vendor "$app/Contents/Resources/jobapps/"
    cp AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
    cp Info.plist "$app/Contents/Info.plist"
fi
# Record the selection so acceptance can reject a mismatched --skip-build app.
/usr/libexec/PlistBuddy -c "Delete :LorcaBuildSDKPath" "$app/Contents/Info.plist" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :LorcaBuildSDKPath string $SDKROOT" "$app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Delete :LorcaBuildSDKVersion" "$app/Contents/Info.plist" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Add :LorcaBuildSDKVersion string $LORCA_SDK_VERSION" "$app/Contents/Info.plist"
# Finder/iCloud can attach metadata to generated bundles that codesign rejects.
xattr -cr "$app"
if [[ "$ui_only" != "1" ]]; then
    codesign --force --sign - "$app/Contents/MacOS/lorca-core"
fi
codesign --force --sign - "$app"
# Verify the signed bundled chat backend in disposable storage. No models or live
# clients are touched, and the check does not depend on deferred Job Apps commands.
if [[ "$ui_only" != "1" ]]; then
python3 - "$app/Contents/MacOS/lorca-core" <<'PYCORE'
import json, os, pathlib, subprocess, sys, tempfile, time, urllib.request

core = str(pathlib.Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='lorca-build-check-') as data:
    environment = dict(os.environ, LORCA_DATA_DIR=data)
    process = subprocess.Popen([core, 'serve'], env=environment,
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE)
    try:
        endpoint_file = pathlib.Path(data) / 'runtime' / 'endpoint.json'
        deadline = time.monotonic() + 8
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        while time.monotonic() < deadline:
            if process.poll() is not None:
                detail = process.stderr.read().decode(errors='replace')[:300]
                raise SystemExit(f'Bundled chat backend failed startup: {detail}')
            if endpoint_file.exists():
                endpoint = json.loads(endpoint_file.read_text())
                request = urllib.request.Request(endpoint['url'] + '/health',
                    headers={'Authorization': 'Bearer ' + endpoint['token']})
                with opener.open(request, timeout=2) as response:
                    health = json.load(response)
                if health.get('protocol') != 3 or health.get('pid') != process.pid:
                    raise SystemExit('Bundled chat backend returned incompatible health data')
                print('Bundled chat backend startup check passed.')
                break
            time.sleep(0.05)
        else:
            raise SystemExit('Bundled chat backend startup check timed out')
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        process.stderr.close()
PYCORE
fi
# lorca rebuild installs with this same toolchain environment and target cache.
# Ordinary app-only builds keep their existing behavior.
if [[ "$ui_only" != "1" && "${LORCA_INSTALL_CLI:-0}" == "1" ]]; then
    (cd .. && cargo install --path . --locked --force --offline --target-dir target)
fi
touch "$app"
echo "Built $PWD/$app"

# Only a successful, signed build may replace running processes. Packaging and
# isolated acceptance tests can opt out explicitly without touching live clients.
if [[ "${LORCA_BUILD_ONLY:-0}" == "1" ]]; then
    echo "Build-only mode: running Lorca was not restarted."
else
    if [[ "$ui_only" == "1" ]]; then
        python3 restart.py "$PWD/$app" --ui-only
    else
        python3 restart.py "$PWD/$app"
    fi
fi

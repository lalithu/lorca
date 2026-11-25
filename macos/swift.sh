#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
source ./toolchain.sh
command="${1:-build}"
shift || true
case "$command" in
    build|test) ;;
    *) echo "Usage: bash macos/swift.sh build|test [Swift options]" >&2; exit 2 ;;
esac
deployment_target="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' Info.plist)"
echo "Swift $command SDK: $SDKROOT ($LORCA_SDK_VERSION)" >&2
exec xcrun swift "$command" --sdk "$SDKROOT" "$@" \
    -Xlinker -platform_version -Xlinker macos -Xlinker "$deployment_target" -Xlinker "$LORCA_SDK_VERSION"

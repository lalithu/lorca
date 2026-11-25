#!/bin/bash
# Source this file so builds, unit tests, and acceptance use the same toolchain.
if [[ -z "${DEVELOPER_DIR:-}" && -d /Applications/Xcode.app ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
if [[ -z "${SDKROOT:-}" ]]; then
    SDKROOT="$(xcrun --sdk macosx --show-sdk-path)" || return 1
    # The CLT 27 SDK lacks the SwiftUI macro plugin on affected installations.
    if [[ "$SDKROOT" == /Library/Developer/CommandLineTools/* && -d /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk ]]; then
        SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
    fi
fi
SDKROOT="$(cd "$SDKROOT" && pwd -P)" || return 1
export SDKROOT
LORCA_SDK_VERSION="$(xcrun --sdk "$SDKROOT" --show-sdk-version)" || return 1
export LORCA_SDK_VERSION

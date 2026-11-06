#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
    echo "Build the portable package on an Apple Silicon Mac." >&2
    exit 1
fi

output="${1:-build}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
cache="${LORCA_RUNTIME_CACHE:-$PWD/build/runtime-cache}"
mkdir -p "$cache"
stage="$(mktemp -d "$output/.lorca-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT

download() {
    local name="$1" url="$2" digest="$3" temporary
    if [[ ! -f "$cache/$name" ]]; then
        temporary="$(mktemp "$cache/.download.XXXXXX")"
        if ! curl --fail --silent --show-error --location --retry 2 --output "$temporary" "$url" ||
           ! printf '%s  %s\n' "$digest" "$temporary" | shasum -a 256 --check --status; then
            rm -f "$temporary"
            echo "Download or checksum verification failed for $name." >&2
            return 1
        fi
        mv -f "$temporary" "$cache/$name"
    fi
    if ! printf '%s  %s\n' "$digest" "$cache/$name" | shasum -a 256 --check --status; then
        echo "Checksum mismatch for $cache/$name. Remove that cached file and retry." >&2
        return 1
    fi
}

ollama_version=v0.40.15
llama_build=b12144
ollama_archive=ollama-darwin.tgz
llama_archive="llama-$llama_build-bin-macos-arm64.tar.gz"
ollama_license="ollama-$ollama_version-LICENSE"
download "$ollama_archive" "https://github.com/ollama/ollama/releases/download/$ollama_version/$ollama_archive" \
    607c79d5d6315ef9d896db83adde736a4c0057196329843544e79b95afb260a8
download "$llama_archive" "https://github.com/ggml-org/llama.cpp/releases/download/$llama_build/$llama_archive" \
    2045e1d2dd7f06beed9a9a7594db1bb1c42e5b92bffcb3bd503679c8d7b6c8a9
download "$ollama_license" "https://raw.githubusercontent.com/ollama/ollama/$ollama_version/LICENSE" \
    5934ed2ce0d15154bcdb9c85203210abac0da4314af34081e36df4599f90b226

app="${LORCA_PACKAGE_APP:-build/lorca.app}"
if [[ -z "${LORCA_PACKAGE_APP:-}" ]]; then
    LORCA_BUILD_ONLY=1 LORCA_UI_ONLY=0 LORCA_INSTALL_CLI=0 bash build.sh
fi
codesign --verify --deep --strict "$app"
version="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")"
bundle="$stage/Lorca"
mkdir -p "$bundle/engines/ollama" "$bundle/engines/llama.cpp"
ditto "$app" "$bundle/lorca.app"
tar -xzf "$cache/$ollama_archive" -C "$bundle/engines/ollama"
tar -xzf "$cache/$llama_archive" -C "$bundle/engines/llama.cpp" --strip-components 1
cp "$cache/$ollama_license" "$bundle/engines/ollama/LICENSE"
cp ../LICENSE "$bundle/LICENSE"
cp ../docs/portable-start.md "$bundle/START_HERE.md"
install -m 755 'Start Lorca.command' "$bundle/Start Lorca.command"
printf 'Ollama %s\nhttps://github.com/ollama/ollama/releases/tag/%s\n\nllama.cpp %s\nhttps://github.com/ggml-org/llama.cpp/releases/tag/%s\n' \
    "$ollama_version" "$ollama_version" "$llama_build" "$llama_build" > "$bundle/engines/VERSIONS.txt"
for binary in "$bundle/lorca.app/Contents/MacOS/lorca" "$bundle/lorca.app/Contents/MacOS/lorca-core" \
              "$bundle/engines/ollama/ollama" "$bundle/engines/llama.cpp/llama-server"; do
    lipo -verify_arch arm64 "$binary"
done

zip="lorca-v$version-macos-arm64-portable.zip"
ditto -c -k --sequesterRsrc --keepParent "$bundle" "$stage/$zip"
(cd "$stage" && shasum -a 256 "$zip" > "$zip.sha256")
mv -f "$stage/$zip" "$stage/$zip.sha256" "$output/"
printf 'Package: %s\nChecksum: %s\n' "$output/$zip" "$output/$zip.sha256"

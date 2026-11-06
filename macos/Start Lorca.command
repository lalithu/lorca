#!/bin/bash
set -euo pipefail
caller="$PWD"
cd "$(dirname "$0")"
root="$PWD"

usage() {
    cat <<'HELP'
Start Lorca.command                  Open Lorca with existing local servers or start bundled Ollama.
Start Lorca.command --ollama MODEL   Pull the named model with Ollama, then open Lorca.
Start Lorca.command --gguf FILE      Serve a local GGUF with bundled llama.cpp, then open Lorca.
HELP
}

case "${1:-}" in
    "") [[ $# == 0 ]] || { usage >&2; exit 2; } ;;
    --help|-h) usage; exit 0 ;;
    --ollama|--gguf) [[ $# == 2 && -n "$2" && "$2" != -* ]] || { usage >&2; exit 2; } ;;
    *) usage >&2; exit 2 ;;
esac

[[ -d "$root/lorca.app" ]] || { echo "Unzip the complete Lorca folder before running this launcher." >&2; exit 1; }

# The launcher only stops the process it starts. Existing servers are shared.
server_pid=""
cleanup() {
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

ready() {
    curl --noproxy '*' -fsS --max-time 1 "$1" >/dev/null 2>&1
}

wait_for_server() {
    local endpoint="$1" attempt
    for ((attempt=0; attempt<60; attempt++)); do
        if ! kill -0 "$server_pid" 2>/dev/null; then
            echo "The server exited during startup. See its output above." >&2
            return 1
        fi
        if ready "$endpoint"; then return 0; fi
        sleep 1
    done
    echo "The local server did not become ready. See its output above." >&2
    return 1
}

ollama_url=http://127.0.0.1:11434/api/tags
ollama="$root/engines/ollama/ollama"
case "${1:-}" in
    --gguf)
        # Resolve relative paths from the caller's directory, including drag-and-drop paths.
        gguf="$2"
        [[ "$gguf" = /* ]] || gguf="$caller/$gguf"
        [[ -f "$gguf" && "$gguf" = *.gguf ]] || { echo "Choose an existing .gguf file." >&2; exit 2; }
        if ready http://127.0.0.1:8080/v1/models; then
            echo "Port 8080 already serves models. Stop that server before serving this GGUF." >&2
            exit 1
        fi
        "$root/engines/llama.cpp/llama-server" --host 127.0.0.1 --port 8080 --model "$gguf" &
        server_pid=$!
        wait_for_server http://127.0.0.1:8080/health
        ;;
    *)
        if ! ready "$ollama_url" && {
            [[ "${1:-}" == --ollama ]] ||
            { ! ready http://127.0.0.1:9931/health && ! ready http://127.0.0.1:8080/health; }
        }; then
            OLLAMA_HOST=127.0.0.1:11434 OLLAMA_NO_CLOUD=1 "$ollama" serve &
            server_pid=$!
            wait_for_server "$ollama_url"
        fi
        if [[ "${1:-}" == --ollama ]]; then
            OLLAMA_HOST=127.0.0.1:11434 "$ollama" pull "$2"
        fi
        ;;
esac

open "$root/lorca.app"
if [[ -n "$server_pid" ]]; then
    echo "Lorca is open. Keep this Terminal window open while using this server. Press Control-C to stop it."
    wait "$server_pid"
fi

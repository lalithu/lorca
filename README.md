# lorca

A compact local Ollama chat client: Rust terminal chat, plus a native macOS
dashboard and floating Mini. Both interfaces use one Rust core; conversations
and images are saved locally.

See [FUNCTIONALITY.md](FUNCTIONALITY.md) for the complete feature reference.

## Install and run

Install [Ollama](https://ollama.com), start it, and download a model:

```sh
ollama serve
# In another terminal:
ollama pull qwen3.5:9b
```

Install lorca from this repository, then launch the built command:

```sh
cargo install --path . --locked --force
lorca
```

```sh
lorca        # Terminal chat (TUI)
lorca main   # macOS dashboard + Mini
```

Build the macOS app once with `./macos/build.sh` before using `lorca main`.
In the GUI, `⌘W` closes the dashboard and shows Mini, even if Mini was hidden.
Use `⌘1` to reopen the dashboard and `⌘Q` to quit both.
See [macOS instructions](macos/README.md). Launching either mode does not build
or print Cargo output.

## Saved chats and images (macOS)

- Create and switch chats in **Chats → Recents**; use `…` to rename or delete.
- Mini’s history menu opens the same chats; drafts and messages stay synchronized.
- Attach images with `+`, drop them on the composer, or paste with `⌘V`.
- Up to four images per message; vision requires a compatible installed model.
- Stored in `~/Library/Application Support/lorca/`: chat JSON in `conversations/`,
  optimized images in `attachments/`. Replies save during streaming and on stop/quit.
- Images: 20 MB input maximum each; optimized to JPEG, at most 4,096 pixels on the
  longest edge and 8 MB each. Requests allow 24 MB of image data across history.

The sections below describe the Rust terminal interface. The GUI keeps its model
and appearance preferences separately in macOS preferences.

## Configuration

On first launch lorca creates `~/.config/lorca/config.json` (or
`$XDG_CONFIG_HOME/lorca/config.json` when set):

```json
{
  "model": "qwen3.5:9b",
  "base_url": "http://127.0.0.1:11434"
}
```

Edit that file to save a different model or Ollama server. Use a model name shown
by `ollama list`. Overrides affect only the current launch:

```sh
lorca --model another-installed-model
lorca --host http://localhost:11434
lorca --config /path/to/config.json
lorca --help
```

On another computer, install Ollama and lorca, download your preferred model,
and set its name in the configuration file. Lorca does not download models or
start Ollama automatically.

## Model lifecycle

Lorca preloads the selected model on launch with an indeterminate animated bar
and elapsed time, then sends `keep_alive: -1` with every chat request to keep it
loaded for the session. The animation shows activity, not a loading percentage.

Switch to another installed model at the input prompt:

```text
/model MODEL_NAME
```

Lorca loads the replacement first, then unloads the previous model. The current
conversation is retained, and saved configuration is unchanged. A failed or
cancelled switch keeps the previous model selected. `/model NAME` with the
already selected model does not reload it.

On `/exit` or double Ctrl+C, lorca requests `keep_alive: 0`. After server-confirmed
unloading, it prints:

```text
● model unloaded
● session closed
```

Ollama shares loaded models across clients and does not provide per-client
ownership information. Lorca warns before every unload because another client
may be using that model. The shared core refuses to unload a model held by
another active lorca session. It cannot track other applications or prevent
Ollama’s scheduler from changing model lifetime.

Loading has a 120-second timeout. Unloading has a five-second request timeout
(with up to one additional second for status verification when needed).
Failures are reported without claiming the model was unloaded. Failed retirements
are retried once when the session exits. A startup loading failure leaves the
prompt available so you can retry with a message or `/model NAME`.

During loading, Esc cancels the operation; two Ctrl+C presses exit. During an
unload, Ctrl+C or Esc skips waiting and reports that cleanup is unconfirmed.

## Conversation

- Enter sends your message. Replies stream into normal terminal scrollback.
- Each submitted message has an elapsed timer that updates while the reply is
  generated and freezes on completion or cancellation. It scrolls naturally with
  the conversation.
- Follow-up messages include the conversation so far.
- The cyan status line above the prompt shows a context usage bar and generation
  tokens/second after each completed reply. Usage includes prompt and generated
  tokens capped at the active window; capacity comes from Ollama's running model, and speed uses its generation
  token count and duration. `--` means unavailable (including cancelled replies).
- Ctrl+C during generation stops the reply and returns to the prompt. Any
  partial reply remains visible and in the conversation context.
- At the prompt, Ctrl+C shows the red exit warning; press it again to exit.
- `/exit` exits. The next terminal launch starts a fresh conversation; earlier terminal chats remain available in the dashboard.

## Architecture

- `src/lib.rs`: reusable Rust core, independent of terminal and SwiftUI rendering.
- `core/`: conversations, generation/cancellation, and per-client model leases.
- `storage.rs`, `types.rs`: compatible JSON, atomic saves, attachment files, and shared data.
- `provider.rs`: Ollama requests, NDJSON streaming, and confirmed model operations.
- `transport.rs`, `client.rs`: authenticated local commands and streamed state snapshots.
- `model.rs`, `lifecycle.rs`, `ui.rs`, `timer.rs`: terminal adapter and presentation; `config.rs`: CLI settings.
- `macos/`: existing SwiftUI/AppKit views; `BackendClient` connects to Rust and `ChatStore` projects its state.

`lorca main` uses the backend bundled by `macos/build.sh`; the CLI uses its own
executable in internal `serve` mode. A process lock makes them reuse one backend.
It binds only to a random port on `127.0.0.1`; a private runtime file supplies its
bearer token. Browser-origin requests are rejected. No service installation is needed.

The backend expires disconnected sessions after 60 seconds and exits 30 seconds
after the last session ends. Unexpected disconnects preserve partial replies and
leave models loaded. The GUI reconnects automatically; the terminal reconnects
before its next model operation or message. Accepted prompts are never replayed
automatically. CLI and GUI retain separate model preferences and client sessions.

Existing macOS JSON, UUIDs, Foundation timestamps, drafts, and image files remain
compatible. Before Rust first writes, it copies original data once to
`~/Library/Application Support/lorca/backups/pre-rust-core/`. Keep that backup until
you have verified your history. `LORCA_DATA_DIR` overrides the data root for testing
or an independent profile; use the same root to share a backend.

## Development checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test --package-path macos
./macos/build.sh
```

The core and transport tests use isolated storage and local test servers; they do
not unload your real Ollama models. There are no tools or agent actions in this version.

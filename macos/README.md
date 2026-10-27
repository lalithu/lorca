# 🫍 lorca for macOS

Native SwiftUI/AppKit dashboard and floating Mini, backed by the existing Rust core.

## Build and launch

Requires macOS 14+, Rust/Cargo, Xcode or Swift command-line tools, and a running Ollama server with an installed model. Python 3 is needed for Job Apps.

From the repository root:

```sh
./macos/build.sh
cargo install --path . --locked --force
lorca main
```

Or run `./macos/run.sh` to build and open the app. Open `macos/Package.swift` in Xcode to edit. Full Xcode is required for `swift test`.

The build script selects full Xcode when installed. On affected command-line-tools installations, its macOS 26.5 SDK fallback avoids a missing SwiftUI macro plugin. The bundle is `macos/build/lorca.app`, containing the Swift executable, Rust core, and Job Apps companion.

## Dashboard and Mini

- Dashboard starts at 880 × 600, remembers its frame, and supports resizing/native full screen.
- `⌘1`: show dashboard; `⌃⌘F`: full screen; `⌘2`: toggle Mini.
- `⌘W`: close dashboard without changing Mini visibility; `⌘Q`: quit both with unload/keep-loaded options.
- Mini starts hidden. Click its emoji to expand/collapse; drag to move. Its controls open the dashboard, create chats, and select history.
- Dashboard and Mini share conversations, drafts, images, selected model, cancellation, and execution status.
- `⌘N`: new chat; `⌘=` / `⌘−`: text size. Settings supports System/Dark/Light appearance.

## Chats

- Recents supports create, open, rename, and delete. Responses render headings, lists, quotes, tables, and code.
- Scroll upward during streaming to pause following; return to the bottom to follow again.
- Select a workspace in the existing composer to enable tools. There is no separate agent chat mode.
- Expand real activity for calls, results, diffs, and terminal output. Approve exact actions once, deny, or cancel; pending approvals stay accessible.
- Mini shows basic task status and links to the dashboard for detailed activity and approvals.
- One conversational run at a time; switching chats does not cancel it. Partial text, structured tools/results, and execution activity persist with the originating chat.
- Reconnection restores backend state without resubmitting work. Backend restarts mark unfinished requests interrupted.

## Model lifecycle

- Default: `qwen3.5:9b` at `http://127.0.0.1:11434`; choose installed models in the composer or Settings.
- Opening the app or selecting a model does not load it. Submitted work prepares it and requests `keep_alive: -1`.
- A temporary indeterminate loading bar and elapsed time appear under `ctx`. Context and speed use confirmed Ollama measurements; unavailable values show `—`.
- **Unload model…** frees memory; **Reload model…** helps recover a stalled model. Stop active work first. Shared-model warnings and Rust session leases apply.
- `⌘Q` offers **Quit & Unload**, **Keep Loaded & Quit**, and **Cancel**, even before a model has loaded.
- Explicit Job Apps research/tailoring can also load a model. Pause research before chatting or switching models.

## Storage and images

- `~/Library/Application Support/lorca/conversations/`: chat JSON; `attachments/`: stored images.
- Streaming checkpoints save about every 200 ms; stop/switch/quit flush. Force-killing can lose the latest checkpoint interval.
- Rust backs up pre-migration data once to `backups/pre-rust-core/`. Unreadable chats remain untouched; back up the whole data folder.
- Add images using `+`, drop, or `⌘V`; click previews to open. Four per message, 20 MB input each, normalized to JPEG up to 4,096 pixels/8 MB; request history allows 24 MB of image data.
- Vision needs a compatible model. Deletion cleans only unreferenced images and pauses cleanup when metadata is unreadable.
- `LORCA_DATA_DIR` selects an independent data root for testing. Conversation IDs and saved data remain compatible.

## Terminal chat

```sh
lorca
lorca --model another-installed-model
lorca --host http://localhost:11434
lorca --config /path/to/config.json
```

Terminal settings live in `~/.config/lorca/config.json` (or `$XDG_CONFIG_HOME/lorca/config.json`):

```json
{"model":"qwen3.5:9b","base_url":"http://127.0.0.1:11434"}
```

The terminal preloads its selected model. `/model NAME` loads the replacement before retiring the old model. Ctrl+C stops generation; at the prompt, the first Ctrl+C shows a red warning and the second exits. `/exit` also exits. Esc cancels loading. Confirmed unloading prints `● model unloaded`, followed by red `● session closed` in natural scrollback. Unload failures are reported without claiming success.

Terminal sessions start fresh; their saved conversations remain accessible in the dashboard. CLI and GUI preferences are separate. Both reuse the authenticated loopback backend, which expires disconnected sessions after 60 seconds and exits 30 seconds after its last client.

## Modules and checks

- `App`, `WindowManager`, `MiniPlacement`: application, menus, native windows, positioning.
- `ChatView`, `ChatComposer`, `ChatInput`, `MarkdownText`: conversation presentation and editing.
- `ChatStore`, `BackendClient`, `ConversationLibrary`: shared state and Rust transport/persistence projection.
- `AgentClient`, `AgentStore`, `AgentActivity*`, `AgentApprovalView`, `AgentDiffView`: live execution UI.
- `JobApps*`, `JobWorkflowView`, `MarkdownNoteEditor`: Job Apps workflow and Notes.

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test --package-path macos
./macos/build.sh
```

[Job Apps setup](../jobapps/README.md) · [Agent contract](../AGENT.md) · [Native acceptance](Acceptance/README.md)

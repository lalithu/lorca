# lorca for macOS

- Requires macOS 14+, Rust/Cargo, Xcode or Swift command-line tools, and a running Ollama server.
- Launch: `cd ~/Downloads/lorca/macos && ./run.sh`
- Launch an existing build: `lorca main` (after installing the Rust command with `cargo install --path . --locked --force` from the repository root).
- Terminal chat: `lorca`. Both commands work from any directory.
- Build only: `./build.sh` → `build/lorca.app` (can be opened in Finder).
- Test: `swift test`. Open `Package.swift` in Xcode to edit.
- Dashboard starts at 880 × 600 and remembers your window size and position. Bringing it forward preserves full screen. **Show Dashboard** (`⌘1`) restores it; **Full Screen Dashboard** (`⌃⌘F`) enters native full screen.
- Click the emoji to open/collapse Mini; drag it to move. **Show / Hide Mini**: `⌘2`.
- Mini’s glass controls: new chat, open dashboard, collapse. Its chat stays anchored beside the pet.
- Blue GUI accents; expanded Mini moves together with the pet and fits back onscreen after dragging.
- Dashboard’s native top bar: toggle sidebar, new chat, current section.
- `⌘W` closes the dashboard and shows Mini if hidden. `⌘1` restores the dashboard; `⌘Q` quits both.
- `⌘=` / `⌘−`: increase / decrease chat text size in Dashboard and Mini.
- Scroll back freely during replies; return to the bottom to resume following. Headings, lists, quotes, tables, and code render as Markdown.
- Both views share saved conversations and drafts. Recents in the sidebar supports create/open/rename/delete; Mini has a history menu. **New Chat**: `⌘N`; stop a reply with Escape or the stop button.
- Both chats show a cyan context bar and response speed from Ollama; unavailable measurements display `—`.
- The selected model preloads at startup and stays loaded (`keep_alive: -1`). A temporary loading bar and timer appear under `ctx` in both chats.
- Model switching offers to unload the old model before loading the next, or explicitly keep both loaded. The core keeps any model still held by another lorca session. A failed replacement tries to restore the previous model. `⌘Q` offers **Quit & Unload** or **Keep Loaded & Quit**, warning that Ollama models are shared with other apps.
- Settings: installed Ollama models and System/Dark/Light appearance. Defaults to `qwen3.5:9b` at `http://127.0.0.1:11434`.
- Overview, Tasks, Pipelines, Scheduled are placeholders.
- Dashboard, Mini, and terminal chat use one Rust backend. The app starts its bundled backend automatically; a lock prevents duplicate runtimes.

## Local storage and images

- Chat JSON: `~/Library/Application Support/lorca/conversations/`; images: `attachments/` alongside it.
- Streaming saves every 200 ms; stop, switching chats, and quit flush immediately. Force-killing can lose the last checkpoint interval.
- Existing saved data stays compatible. Original files are backed up once to `backups/pre-rust-core/`; unreadable chats remain untouched. Keep the whole `lorca` folder when backing up or restoring.
- Add images with `+`, drop on the composer, or paste (`⌘V`); click a preview to open the stored image.
- Four images/message, 20 MB input/image. Images normalize to JPEG (maximum 4,096-pixel edge, 8 MB each); total request image data is limited to 24 MB.
- A vision-capable model is required for chats containing images. Missing images or unsupported models produce a friendly error.
- Deleting a chat or draft image removes only files unused by other saved chats. Cleanup pauses if any chat metadata is unreadable.

## Files

- `App.swift`, `WindowManager.swift`: application, menus, desktop/full-screen and Mini windows.
- `DashboardView.swift`, `MiniView.swift`, `ChatView.swift`: native views and basic Markdown.
- `ChatStore.swift`, `ModelSession.swift`, `BackendClient.swift`: UI state, loading presentation, authenticated Rust connection.
- `Resources/orca.txt`: copy of the repository’s existing emoji asset.
- `ConversationLibrary.swift`, `Conversation.swift`: observable history projection and compatible data types. Rust owns persistence.
- `ChatComposer.swift`, `AttachmentStrip.swift`, `ImageInput.swift`, `AttachmentStore.swift`: native image input, JPEG normalization, and read-only previews. Rust validates and stores attachments.
- `ChatInput.swift`, `ClipboardImages.swift`: native text editing and image paste/drop handling.

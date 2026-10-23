# lorca for macOS

- Requires macOS 14+, Xcode or Swift command-line tools, and a running Ollama server.
- Launch: `cd ~/Downloads/lorca/macos && ./run.sh`
- Launch an existing build: `lorca main` (after installing the Rust command with `cargo install --path . --locked --force` from the repository root).
- Terminal chat: `lorca`. Both commands work from any directory.
- Build only: `./build.sh` → `build/lorca.app` (can be opened in Finder).
- Test: `swift test`. Open `Package.swift` in Xcode to edit.
- Dashboard opens as a normal, resizable 880 × 600 window. **Show Dashboard** (`⌘1`) restores it; **Full Screen Dashboard** (`⌃⌘F`) enters native full screen.
- Click the emoji to open/collapse Mini; drag it to move. **Show / Hide Mini**: `⌘2`.
- Mini’s glass controls: new chat, open dashboard, collapse. Its chat stays anchored beside the pet.
- Blue GUI accents; expanded Mini moves together with the pet and fits back onscreen after dragging.
- Dashboard’s native top bar: toggle sidebar, new chat, current section.
- `⌘W` closes the dashboard and shows Mini if hidden. `⌘1` restores the dashboard; `⌘Q` quits both.
- `⌘=` / `⌘−`: increase / decrease chat text size in Dashboard and Mini.
- Scroll back freely during replies; return to the bottom to resume following. Headings, lists, quotes, tables, and code render as Markdown.
- Both views share one in-memory conversation. **New Chat**: `⌘N`; stop a reply with Escape or the stop button.
- Both chats show a cyan context bar and response speed from Ollama; unavailable measurements display `—`.
- The selected model preloads at startup and stays loaded (`keep_alive: -1`). A temporary loading bar and timer appear under `ctx` in both chats.
- Model switching preloads the replacement, then offers to unload the previous model. `⌘Q` offers **Quit & Unload** or **Keep Loaded & Quit**, warning that Ollama models are shared with other apps.
- Settings: installed Ollama models and System/Dark/Light appearance. Defaults to `qwen3.5:9b` at `http://127.0.0.1:11434`.
- Overview, Tasks, Pipelines, Scheduled are placeholders.
- Rust terminal chat is preserved; `lorca main` launches this app. This prototype talks directly to Ollama; the Rust-core bridge comes later.

## Files

- `App.swift`, `WindowManager.swift`: application, menus, desktop/full-screen and Mini windows.
- `DashboardView.swift`, `MiniView.swift`, `ChatView.swift`: native views and basic Markdown.
- `ChatStore.swift`, `ModelSession.swift`, `OllamaClient.swift`: shared chat, session model lifecycle, Ollama requests.
- `Resources/orca.txt`: copy of the repository’s existing emoji asset.

<div align="center">

# 🫍 lorca

**A local AI workspace. One Rust core. Native on your Mac.**

Chat, work on code, and manage job applications—with a floating orca at your side.

**v0.4.0 · Rust · SwiftUI · AppKit · Ollama · Python**

[Quick start](#quick-start) · [Features](FUNCTIONALITY.md) · [macOS](macos/README.md) · [Agent runtime](AGENT.md) · [Job Apps](jobapps/README.md)

</div>

---

## Features

- **Chats:** streamed responses, Markdown, images, saved conversations, and a shared Dashboard/Mini experience.
- **Coding:** workspace tools, live activity, readable diffs, terminal output, and exact-action approvals in the same conversation.
- **Mini:** a draggable floating orca with a compact chat panel, shared history, and quick access to the dashboard.
- **Job Apps:** evidence-backed research, guided applications, Gmail drafts with résumé attachments, reply tracking, and automatic follow-up dates.
- **Local models:** installed Ollama models, context and speed metrics, cancellation, and manual unload/reload controls.
- **Two Macs:** optional Google Sheets sync for Job Apps records, profile, templates, résumé source, and approved master PDF.
- **Runtime:** authenticated loopback API, NDJSON event streaming, atomic JSON saves, SQLite job state, per-session model leases, and exact-action permission tokens.

Built as a personal workspace by **Lalith Uriti**. The native app is an evolving prototype; coding tools and integrations have deliberate limits.

## Quick start

Requires **macOS 14+**, Rust/Cargo, Xcode or Swift command-line tools, and [Ollama](https://ollama.com). Job Apps also requires Python 3. Full Xcode is needed for Swift tests.

Start Ollama and install a model, then build lorca:

```sh
ollama pull qwen3.5:9b  # Keep the Ollama app/server running.
git clone https://github.com/lalithu/lorca.git
cd lorca
cargo install --path . --locked --force
./macos/build.sh
```

```sh
lorca main   # Native dashboard + optional floating Mini
lorca        # Terminal chat
```

The app bundle is also available at `macos/build/lorca.app`. Launching an existing build does not run Cargo. Models are installed separately; lorca does not download them or start Ollama.

## Chat and code

1. Open **Chats** and choose an installed model in the composer.
2. Send a question or attach images with `+`, drag-and-drop, or paste. Vision needs a compatible model.
3. For coding tasks, select a workspace in the same composer. Expand live activity to inspect actual tools, changes, and output.
4. Review each proposed edit or command, then **Allow Once**, **Deny**, or **Cancel Task**.

Scroll upward to read while lorca works; return to the bottom to resume following. Chats, drafts, images, partial replies, and execution activity save locally. Mini shares the same conversation state.

| Shortcut | Action |
| --- | --- |
| `⌘N` | New chat |
| `⌘1` / `⌘2` | Show dashboard / toggle Mini |
| `⌘W` | Close dashboard; Mini keeps its current visibility |
| `⌘Q` | Quit with model unload/keep-loaded options |
| `⌘=` / `⌘−` | Increase / decrease chat text size |
| `Esc` | Stop a response |

The GUI loads a model when work starts, rather than on launch, and requests `keep_alive: -1`. Manual unload/reload warns about other Ollama clients; Rust protects other lorca sessions. The terminal preloads its model and uses two-step Ctrl+C to exit. See [model lifecycle and terminal controls](macos/README.md#terminal-chat).

## Job Apps

Work through **Apply → Recruiter → Review in Gmail → Waiting**. Lorca prepares a draft with a `Role @ Company` subject, your filled template, and the approved master résumé. You review and send in Gmail. Confirmed sends schedule follow-up five weekdays later; replies return the job to your attention.

Google setup is optional and separate from local chat. See [setup, sync, and limitations](jobapps/README.md).

## How it fits together

```text
SwiftUI Dashboard + Mini ─┐
                         ├─ Rust core ─ Ollama
Rust terminal client ────┘    ├─ conversations, images, model leases
                             └─ agent loop, tools, permissions

Job Apps SwiftUI ─ Python companion ─ SQLite / Google Sheets / Gmail
                     └─ bundled Rust research session
```

| Directory | Responsibility |
| --- | --- |
| `src/core/`, `src/agent/` | Shared conversation runtime, execution, and permissions |
| `src/provider/`, `src/context.rs` | Ollama streaming, tools, and working context |
| `src/storage.rs`, `src/transport.rs` | Atomic local persistence and authenticated loopback transport |
| `macos/` | Native views, shared state, Mini, and app packaging |
| `jobapps/` | Research, application workflow, Google integrations, and sync |

One active conversational run at a time. Tools are bounded; commands support a limited Cargo allowlist under the macOS sandbox. Working context compacts automatically while saved history remains intact. Interrupted tasks are not automatically replayed. [Runtime contract and limits →](AGENT.md)

## Data and privacy

- Chats and Job Apps data live under `~/Library/Application Support/lorca/`, outside the repository.
- Chat/model inference uses your configured Ollama server. Job research uses external search/extraction; Gmail and Sheets require Google access.
- Optional Sheets sync contains personal Job Apps data, including the approved résumé PDF. Use a private Sheet and one active Mac at a time.
- OAuth credentials, API keys, local trackers, résumé files, and templates are excluded from Git. This personal build includes the author's Gmail address in its Job Apps integration.
- Chat history does **not** sync through Sheets. Keep a backup of the Application Support folder.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
python3 -B -m unittest discover -s jobapps/tests
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcrun swift test --package-path macos
./macos/build.sh
```

After pulling updates, rebuild the app and quit/reopen lorca. To update the installed terminal command, rerun `cargo install --path . --locked --force`. Google credentials remain local on each Mac; Job Apps data travels through the configured Sheet.

## Versions and direction

Current package version: **0.4.0**. Future milestones below are plans, not shipped features or release dates.

| Version | Focus |
| --- | --- |
| **0.4.x · current** | Unified Rust core, native Chats/Mini, local coding tools, persistence/vision, and Job Apps; continued reliability and polish |
| **0.5 · planned** | MCP integrations using the existing runtime and permission boundaries |
| **0.6 · planned** | A2A (Agent-to-Agent) interface and scoped collaboration |

The direction stays the same: a modular Rust execution core, native macOS UI, local models, and visible user control over actions.

---

[Apache 2.0](LICENSE) · Copyright © 2026 Lalith Uriti · [Third-party attribution](jobapps/vendor/NOTICE.md)

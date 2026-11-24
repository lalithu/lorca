# Lorca documentation 🌸

Lorca pairs a local model with a Rust harness for context, memory, tool execution,
and permission checks. The model chooses actions and when to finish.

## What is available

- Native macOS dashboard, floating Mini, and terminal chat through a shared Rust core.
- Saved conversations, drafts, streamed replies, images, and tool activity; private reasoning is ephemeral.
- File inspection and edits, approved terminal commands, history recall, and configurable web search.
- Ollama and llama.cpp model selection in the GUI; the CLI uses Ollama. Vision depends on the model.
- Job Apps research, résumé preparation, reviewed Gmail actions, and optional private Sheets sync.
- Editable Notes with local model contributions, image support, and source-backed search.

One conversational run is active at a time. Tool discovery does not grant access;
workspace boundaries and approvals still apply.

## Guides

| Guide | Contents |
| --- | --- |
| [Quick start](../README.md#quick-start) | Install and launch |
| [macOS](../macos/README.md) | Build, controls, models, and local storage |
| [Portable setup](portable-start.md) | Package the app and runtimes; start Ollama or a GGUF |
| [Architecture](architecture.md) | Model input, execution loop, memory, and context |
| [Tools](tools.md) | Schemas, discovery, permissions, and limitations |
| [Contributor guide](../AGENT.md) | Checks and operational boundaries |
| [Job Apps](../jobapps/README.md) | Research, Gmail, and Sheets setup |
| [Notes](notes-following.md) · [Notes internals](ems-notes.md) | Editing workflow and implementation |
| [Roadmap](roadmap.md) | Upcoming work |
| [v0.5.0 release](releases/v0.5.0.md) · [Validation](v0.5-validation.md) | Published release evidence and limitations |
| [Native acceptance](../macos/Acceptance/README.md) | macOS acceptance checks |

## Repository map

| Path | Purpose |
| --- | --- |
| `src/` · `tests/` | Rust runtime, providers, tools, and tests |
| `prompts/system.md` | Model-facing system prompt, embedded at build time |
| `macos/` | Native UI, build scripts, and packaging |
| `jobapps/` | Job Apps companion and tests |
| `docs/` · `assets/` | Documentation, diagrams, screenshots, and app assets |
| `.github/` | CI and repository workflows |
| `target/` | Generated Rust build cache, ignored by Git |

Chats, settings, and credentials live outside the repository. See the setup guides
for storage paths; use a private Sheet for optional Job Apps sync.

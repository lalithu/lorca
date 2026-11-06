# 🫍 lorca · functionality

- **Rebuild:** `lorca rebuild` updates core, CLI, and GUI, then opens the dashboard; native 🌊 app icon. `lorca rebuild ui` rebuilds only Swift and relaunches the UI.
- **Interfaces:** Rust terminal chat, resizable native dashboard, floating Mini; shared Rust core.
- **Chats:** create, rename, switch, delete; saved messages, drafts, partial replies, and tool activity.
- **Responses:** streaming Markdown, free scrolling with bottom-follow, text zoom, cancellation.
- **Images:** picker, drop, paste, previews, persistence; compatible local vision models (provider-dependent).
- **Coding:** optional workspace, file discovery/reads/search, proposed patches, bounded Cargo commands, diffs/output, per-action approvals.
- **Models:** selection, lazy GUI loading, terminal preload, session keep-alive, confirmed unload/reload, shared-session protection.
- **Metrics:** context usage, allocated capacity, tokens/second, loading and response timers.
- **Terminal:** compact ASCII orca, scrollback, `/model`, `/exit`, two-step Ctrl+C, restored terminal state.
- **Job Apps:** source-backed research, guided application/outreach queue, recruiter contacts, live Markdown notes, résumé tailoring.
- **Gmail:** filled templates, automatic subjects, résumé drafts, confirmed send/reply tracking, five-weekday follow-up dates; manual sending.
- **Cross-Mac:** Job Apps records, profile, preferences, templates, LaTeX source, approved master PDF via private Google Sheets; conflict review.
- **Recovery:** atomic chat saves, authenticated local transport, reconnect without replay, interrupted task handling.
- **Limits:** one conversational run, bounded tools; GUI supports llama.cpp with Ollama fallback, while the CLI uses Ollama; chat history and Google credentials remain local. Overview is a placeholder.

[Setup](README.md) · [macOS](macos/README.md) · [Agent limits](AGENT.md) · [Job Apps](jobapps/README.md)

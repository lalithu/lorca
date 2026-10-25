# lorca functionality

- `lorca`: terminal chat; `lorca main`: macOS dashboard + Mini.
- GUI chat history: create, switch, rename, delete, and restore local conversations.
- Images: picker, drag/drop, paste, previews, local storage, and Ollama vision.
- Shared Dashboard/Mini chats, Markdown, manual scrolling, and `⌘=` / `⌘−` text sizing.
- One Rust core for both interfaces: saved chats, images, Ollama, streaming, and cancellation.
- Terminal sessions start fresh; their history remains available in the dashboard.
- Automatic model preloading; animated loading bar with elapsed time.
- Models stay loaded during the session (`keep_alive: -1`).
- `/model NAME`: load a replacement, then unload the previous model.
- `/exit` or double Ctrl+C: unload the model and close the session.
- Ctrl+C stops generation; Esc cancels loading.
- Shared-model warnings; lorca session leases prevent unloading another session’s model.
- Authenticated local backend, automatic startup/reuse, reconnect, and original-data backup.
- Unload success requires server confirmation.
- Cyan context bar: usage percentage, token count, capacity, and tokens/second.
- Live elapsed timer beside each submitted message.
- ASCII orca, compact header, title emoji, bold amber labels, natural scrollback.
- Saved settings in `~/.config/lorca/config.json` (supports `XDG_CONFIG_HOME`).
- Launch overrides: `--model`, `--host`, `--config`; help: `--help`.
- Error recovery, cancellable loading/unloading, and terminal-state restoration.
- Confirmed `● model unloaded`, followed by red `● session closed`.

- One Chats composer: conversations, images, optional workspace tools, streamed text, live diffs/output, exact-action approvals, and cancellation.
- Requests, partial replies, structured tools/results, and activity saved in the same chat; Mini shares status; reconnect never replays tasks.
- Automatic model-context compaction; confirmed per-turn usage and actual allocated capacity.
- Discovery-backed file references, revalidated across follow-ups; bounded read excerpts can expand via nextOffset.
- `lorca agent --workspace PATH "TASK"`: native tool loop, explicit action approvals, cancellation, and live events ([details](AGENT.md)).

- Model menu → **Reload model…**: unload/reload a stalled model after a shared-model warning. Stop active work first; Rust protects other lorca sessions.

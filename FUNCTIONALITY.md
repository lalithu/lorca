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

- `lorca agent --workspace PATH "TASK"`: native tool loop, explicit action approvals, cancellation, and live events ([details](AGENT.md)).

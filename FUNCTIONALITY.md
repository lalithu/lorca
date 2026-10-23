# lorca functionality

- Local Ollama chat with streaming replies and session-only conversation memory.
- Automatic model preloading; animated loading bar with elapsed time.
- Models stay loaded during the session (`keep_alive: -1`).
- `/model NAME`: load a replacement, then unload the previous model.
- `/exit` or double Ctrl+C: unload the model and close the session.
- Ctrl+C stops generation; Esc cancels loading.
- Shared-model warnings; unload success requires server confirmation.
- Cyan context bar: usage percentage, token count, capacity, and tokens/second.
- Live elapsed timer beside each submitted message.
- ASCII orca, compact header, title emoji, bold amber labels, natural scrollback.
- Saved settings in `~/.config/lorca/config.json` (supports `XDG_CONFIG_HOME`).
- Launch overrides: `--model`, `--host`, `--config`; help: `--help`.
- Error recovery, cancellable loading/unloading, and terminal-state restoration.
- Confirmed `● model unloaded`, followed by red `● session closed`.

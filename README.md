# lorca

A compact terminal chat client for local Ollama models. Conversations stay in
memory for the current session.

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
may be using that model. Its requests cannot prevent other clients or Ollama's
scheduler from changing a model's lifetime.

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
- `/exit` exits. The next launch starts a fresh conversation.

## Code structure

- `main`: startup and wiring.
- `config`: saved settings and launch flag overrides.
- `model`: Ollama HTTP requests, streamed replies, cancellation, and model operations.
- `lifecycle`: preload/switch/unload orchestration, progress animation, and shared-model warnings.
- `chat`: session message history.
- `ui`: terminal rendering and keyboard input.

There are no tools, file access, or agent actions in this version.

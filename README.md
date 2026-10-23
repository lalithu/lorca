# lorca

A compact terminal chat client for local Ollama models. Conversations stay in
memory for the current session.

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

Launching `lorca` runs the installed binary without Cargo output.

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
- `model`: Ollama HTTP requests, streamed replies, and cancellation.
- `chat`: session message history.
- `ui`: terminal rendering and keyboard input.

There are no tools, file access, or agent actions in this version.

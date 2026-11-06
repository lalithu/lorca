# 🌸 Lorca 🫍

Lorca is a local AI agent runtime that adapts to your hardware and uses persistent memory to carry work across model calls.

![Lorca architecture showing native and terminal clients, the Rust core, the two local model servers, and local data](docs/architecture.svg)

The Rust core routes requests to the model you select. The model chooses tools and
when to finish. Rust checks permissions, executes approved actions and records
what happened. [Architecture and four pillars](docs/architecture.md)

I started Lorca to learn how agent harnesses and local inference work. It became an agent I use for my own work.

## Upcoming work

- **MCP** for connecting external tools and resources.
- **More robust native tools** for everyday workspace and agent workflows.
- **Leaner tool schemas** with clearer inputs and fewer tokens per declaration.
- **Broader runtime testing** across models and Apple Silicon Macs.


## Quick start

- Already running Ollama or llama.cpp? Download the [Apple Silicon app](https://github.com/lalithu/lorca/releases/tag/v0.5.0).
- Need both runtimes? [Build the portable ZIP](#build-from-source), unzip it, and open **Start Lorca.command**.

To download a starter model and launch the portable app, run this from the unzipped folder:

```sh
./"Start Lorca.command" --ollama qwen3:4b
```

Pick your model in the composer. See the [portable setup guide](docs/portable-start.md) for GGUF setup.


```sh
lorca             # Open terminal chat
lorca main        # Open the macOS app
lorca rebuild ui  # Rebuild only the UI
lorca rebuild     # Rebuild the full app
```


## Documentation

[Documentation](docs/) covers releases, runtime, tools, and setup. Lorca stores data locally, supports optional private Sheets sync for Job Apps, and is licensed under Apache 2.0.

[Apache 2.0](LICENSE)

Credits: [llama.cpp](https://github.com/ggml-org/llama.cpp), [Ollama](https://github.com/ollama/ollama), [OpenAI Codex](https://openai.com/codex/), [Claude Code](https://www.anthropic.com/claude-code)


# 🌸 Start Lorca on an Apple Silicon Mac

Unzip the download and keep the entire **Lorca** folder together. It contains
the app, Ollama, llama.cpp, and their licenses. You can run it from Downloads or
move the folder to Applications. Python 3 is needed only for Job Apps.

Double-click **Start Lorca.command**. Lorca uses local servers you already
started. If it finds neither server, the launcher starts bundled Ollama on
`127.0.0.1:11434`. Pick an available model from the menu next to the send button.

## First model

In Terminal, change into the unzipped Lorca folder. This command explicitly
downloads [Qwen3 4B](https://ollama.com/library/qwen3:4b), about 2.5 GB, and opens Lorca.

```sh
./"Start Lorca.command" --ollama qwen3:4b
```

For a GGUF model you already have, pass its path to bundled llama.cpp.

```sh
./"Start Lorca.command" --gguf "/path/to/model.gguf"
```

This starts llama.cpp on `127.0.0.1:8080`. You can keep an Ollama server running
alongside it. Choose **Refresh models** in the composer if a server starts
after Lorca opens. Keep the launcher’s Terminal window open for any server it
starts. Press Control-C in that window to stop that server.

For terminal chat with Ollama, run `./lorca.app/Contents/MacOS/lorca-core`
from the unzipped folder.

The ZIP contains no model weights. Ollama keeps its regular model cache.
Lorca keeps chats and settings in `~/Library/Application Support/lorca`.
The app is ad-hoc signed for local testing. A public macOS distribution still
needs a Developer ID signature and notarization for the normal Gatekeeper flow.

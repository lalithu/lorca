# Roadmap

Planned work, not a list of implemented capabilities:

- **MCP:** connect external tools and resources.
- **Tools and schemas:** improve native tools, concise declarations, and actionable failure feedback.
- **Permissions:** replace wording-specific scope inference with explicit authority state while preserving user control.
- **Sustained work:** distinguish inactivity from operation deadlines and define ownership for longer-lived commands.
- **Provider compatibility:** improve capability discovery and isolate model-specific handling in the inference layer. The GUI already supports Ollama and llama.cpp; the CLI uses Ollama.
- **Validation:** broaden model and Apple Silicon coverage, including recovery, context allocation, and native tool calling.

Keep changes small: the model owns reasoning and tool selection; the harness owns
execution, context, memory, and permission enforcement.

### Model & Provider Agnosticism

**Goal:** Make Lorca fully model- and provider-agnostic, allowing seamless switching between models and inference backends without modifying the agent harness.

**Current State:** The five tool drawers, four-pillar harness, permissions, and verification are model-independent, but inference currently relies on Ollama.

**Future Improvements:**
- Support multiple inference backends, including Ollama, llama.cpp, and MLX.
- Automatically detect model capabilities (vision, tool calling, reasoning).
- Keep model-specific behavior and error handling isolated in the inference layer.
- Eliminate Qwen-specific assumptions from the agent harness.
- Ensure all models use the same tools, permissions, and verification system.
- Adapt tool schemas and generation settings to each model's capabilities.

**Core Principle:** The harness adapts to the model, not the other way around.
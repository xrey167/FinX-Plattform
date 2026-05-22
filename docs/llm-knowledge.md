# LLM And Knowledge Intelligence

G006 keeps the model and retrieval layer deliberately small:

- `tdw-llm` owns the in-house `LanguageModel` trait, chat request/response
  types, usage accounting, and config-derived model selection.
- `tdw-llm-anthropic` and `tdw-llm-openai-compat` provide deterministic adapter
  contracts for Anthropic Messages-style and OpenAI-compatible providers. They
  do not perform network calls in tests.
- `tdw-knowledge` indexes documents through the existing local embedding
  provider, in-memory Qdrant-compatible vector engine, KG, and tag store.
- `tdw-knowledge::summarize_syntax` provides the first syntactic context summary
  for schemas and code without taking a dependency on an LSP or large agent
  framework.

`tdw-service-api::llm_knowledge_sample` wires the model adapter, retrieval
index, active tags, and syntax summary into one service-facing evidence path.

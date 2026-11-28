# Local retrieval-augmented generation (RAG)

Good Search can answer questions using passages in its local knowledge index. Retrieval uses local FTS5/lexical search and, when enabled, MiniLM/CLIP semantic channels. Retrieved context is sent to a local Ollama server. The default workflow does not require a cloud model provider.

## Requirements

- Ollama running at `http://127.0.0.1:11434`, or the endpoint configured in `[answer].endpoint`.
- The selected model installed in Ollama.
- An existing Good Search index containing extracted document or captured web content.

~~~sh
ollama list
ollama pull qwen3.5:2b-q4_K_M
~~~

The default model is configurable. Choose a model that fits available memory and context requirements.

## Ask a question

~~~sh
gsearch answer "What is a throttling device?"
gsearch answer --model qwen3.5:2b-q4_K_M "Explain the throttling process"
gsearch answer --browser "What is a throttling device?"
~~~

The command returns a structured result containing status, answer and source references. Sources identify the indexed item and, when available, a PDF page or text location. The model is instructed to use retrieved excerpts and identify when the supplied context is insufficient.

The frontend uses the same answer pipeline and stores messages in the selected conversation. It supports multi-turn context, incremental output, regeneration and retrying failed answers. Search remains independent of generation: ordinary search does not start Ollama.

## Retrieval modes and semantic indexing

The default retrieval path is lexical/FTS5 and does not download embedding models. Semantic retrieval uses local vectors after semantic indexing has been enabled and the index populated:

~~~sh
gsearch config set indexing.embeddings_enabled true
gsearch index embed
gsearch query "heat exchanger" --semantic
~~~

Future syncs maintain vectors while semantic indexing remains enabled. First use downloads selected model files; inference then runs locally. See [Model setup](MODELS.md).

RAG quality depends on indexed content completeness and the relevance of retrieved passages. Semantic search can improve retrieval for some queries but does not guarantee that the right evidence will be found.

## Source grounding

The chat interface exposes a per-message source-grounding control. When enabled, Good Search asks the model to answer from retrieved context and attach source references. When disabled, the model can answer without the same source-grounding constraint. This is a mode switch, not a truth detector: answers can still be incomplete, misread evidence or contain errors.

Citations point to locally indexed sources. They do not independently prove that a generated claim is correct; inspect the referenced passage when accuracy matters.

## Thinking models and streaming

Good Search checks Ollama model metadata for thinking capability before sending thinking-specific request options. If the model does not advertise that capability, the request avoids the option. If Ollama rejects the option, Good Search retries without it.

Thinking content is kept separate from user-facing answer text. If a model emits only thinking content without an answer, Good Search can retry with a larger output budget within configured safety bounds.

When streaming is enabled, Ollama's incremental response is forwarded through Good Search's answer job API and displayed progressively in the frontend. The job API uses sequence cursors so clients can request only new output and recover with a snapshot if an older cursor falls outside the retained delta window. A failed job may retain and display partial generated text.

## Failure states and resource considerations

If Ollama is unavailable or rejects a request, Good Search reports the underlying connection/API error. If no indexed passages match, the result reports that no context was found. Normal file and knowledge search remains usable when answer generation is unavailable.

Small local models reduce memory pressure but can produce weaker answers. Larger models and longer context windows use more RAM and CPU. Keep context and output limits bounded on machines with limited memory. Model selection does not remove the need to inspect source material or verify important claims.

## Browser-ready answers

The `gsearch answer --browser` workflow writes a local HTML answer page and opens it in the default browser. The page contains the answer and source links. This workflow is separate from the interactive frontend, which stores the answer in a conversation.

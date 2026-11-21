# Local RAG answers

Hydrogen can answer a question from passages already present in its local
document index. Retrieval uses Hydrogen's existing FTS5/lexical search and,
when explicitly requested, the already-enabled MiniLM/CLIP semantic channels.
The retrieved passages are sent to a local Ollama server; no cloud model or
external API is used.

## Requirements

- Ollama running locally at `http://127.0.0.1:11434`.
- The model tag installed in Ollama. The default is `gemma3:1b`.
- An existing Hydrogen index containing extracted document text.

Check the model with:

```sh
ollama list
ollama run gemma3:1b
```

## Ask a question

```sh
hydrogen answer "what is a throttling device"
```

Optional flags:

```sh
hydrogen answer "explain the throttling process" --limit 6 --model gemma3:1b
hydrogen answer "explain the throttling process" --semantic
```

The command prints one JSON object containing `status`, `answer`, and
`sources`. Each source includes its indexed path and PDF page or text line
when available. The model is instructed to answer only from retrieved
excerpts, cite claims as `[1]`, `[2]`, and say when the indexed material
does not contain enough information.

The default retrieval path is deliberately lexical/FTS5 and does not download
embedding models. `--semantic` uses the optional local vectors only if they
have already been enabled and indexed. RAG answers use the stored search
snippets, so answer quality depends on the completeness of indexed text and
the quality of the retrieved passages.

If Ollama is unavailable or the requested model cannot be used, the command
returns `status: "unavailable"`; if no indexed passages match, it returns
`status: "no_context"`. Normal file search remains independent of answer
generation.

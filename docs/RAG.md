# Local RAG answers

Hydrogen can answer a question from passages already present in its local
document index. Retrieval uses Hydrogen's existing FTS5/lexical search and,
when explicitly requested, the already-enabled MiniLM/CLIP semantic channels.
The retrieved passages are sent to a local Ollama server; no cloud model or
external API is used.

## Requirements

- Ollama running locally at `http://127.0.0.1:11434`.
- The model tag installed in Ollama. The default is `qwen3.5:2b-q4_K_M`.
- An existing Hydrogen index containing extracted document text.

Check the model with:

```sh
ollama list
ollama run qwen3.5:2b-q4_K_M
```

## Ask a question

```sh
hydrogen answer "what is a throttling device" --model qwen3.5:2b-q4_K_M

# Also write a browser-ready local HTML answer page
hydrogen answer --browser "what is a throttling device"
```

Optional flags:

```sh
hydrogen answer "explain the throttling process" --limit 8 --model qwen3.5:2b-q4_K_M
hydrogen answer "explain the throttling process"
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


## Passage-level retrieval

The RAG command searches the PDF page FTS index directly, so multiple relevant pages from the same PDF can be retrieved. It expands a question into a small set of keyword queries, combines repeated page hits, and supplements them with indexed non-PDF text. Image files and screenshot OCR are excluded from answer context by default. This is intentionally separate from the launcher’s broad content-and-vision search ranking.

The default generator is `qwen3.5:2b-q4_K_M` (about 1.9 GB in the Ollama library). On an 8 GB machine, keep the context bounded; larger models may compete with the desktop and browser for memory. A model change does not guarantee zero hallucinations: the retrieval layer and citation checks remain essential.


## Browser-ready answers

The `hydrogen answer` command remains opt-in. Use `--html` to write a
self-contained, locally stored HTML page under
`$HYDROGEN_STATE_DIR/answers` (or the configured Hydrogen state directory).
The JSON response includes `html_path` when the page is written. Source entries
link to their original local files and include PDF page fragments where available.
The Quickshell launcher adds an explicit **Ask AI about this search** result;
searching alone never invokes Ollama. Selecting that result runs RAG and opens
the generated page with `xdg-open`.

If Ollama is unavailable or rejects the requested model, Hydrogen now returns
the underlying connection/API error rather than collapsing every failure into
a generic unavailable message.

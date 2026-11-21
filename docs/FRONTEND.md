# Hydrogen frontend

The bundled frontend is served by the Hydrogen binary. It uses the configured SQLite database and Ollama model.

## Start

```sh
hydrogen --frontend
```

The default address is `http://127.0.0.1:17843`. Hydrogen only binds to loopback addresses. If the service is already running, the command opens the existing instance.

Open a new chat with a question:

```sh
hydrogen --frontend --ask "What is a throttling device?"
```

Open a saved conversation:

```sh
hydrogen --frontend --conversation 12
```

Open a saved offline article:

```sh
hydrogen --frontend --page 8
```

Generate a standalone HTML answer and open it in the default browser:

```sh
hydrogen answer --browser "What is a throttling device?"
```

## Features

- Persistent local conversation history.
- Source-grounded answers using the configured Ollama model.
- Search across local files, saved conversations and captured web pages.
- Offline article reading view.
- Editable Hydrogen settings.
- Local browser-capture API.

Ordinary search does not start inference. The Ask Hydrogen action explicitly opens the frontend and starts a conversation.

## Configuration

The frontend settings are stored in the same `~/.config/hydrogen/config.toml` file as indexing and model settings.

Relevant keys:

- `frontend.enabled`, `frontend.bind`, `frontend.port`, `frontend.auto_open`
- `answer.model`, `answer.retrieval_limit`, `answer.context_window`
- `web.enabled`, `web.auto_index`, `web.minimum_text_length`
- `web.minimum_paragraphs`, `web.allow_domains`, `web.exclude_domains`
- `conversations.enabled`, `conversations.capture_ai_chats`

Settings changed through the frontend are written through the same validated configuration setter used by `hydrogen config set`. Changing the listening port requires restarting the frontend service.

## Data

The frontend stores conversations, messages, captured web pages, visit records and content versions in the configured Hydrogen SQLite database. The schema migration is additive; existing file and PDF indexes are retained.

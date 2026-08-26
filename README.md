# Ariadne

Ariadne is an open-source, local-first AI agent built with Rust, React, and Tauri. One shared application core powers an interactive CLI, deterministic one-shot jobs, a long-running HTTP service, a browser UI, and a native desktop app.

> **Project status:** bootstrap foundation. The model-provider path, versioned profiles, and all product surfaces are working, but skill execution, MCP tool execution, durable memory, approvals, and long-running agent loops are intentionally future capabilities.

## Why Ariadne

- **Local by default:** connects to an OpenAI-compatible endpoint at local Ollama by default.
- **Automation friendly:** `ariadne run` reads a flag or stdin and supports machine-readable JSON.
- **VPS ready:** `ariadne serve` is stateless, handles graceful shutdown, and can serve the web build from the same binary.
- **One core, several surfaces:** HTTP, terminal, and Tauri code remain thin adapters around `ariadne-core`.
- **Provider portable:** use Ollama locally or set environment variables for another OpenAI-compatible API.
- **Profile scoped:** local, work, automation, and hosted profiles can select different providers, models, system prompts, active skills, and MCP servers.

## Repository layout

```text
apps/
  cli/                 Rust CLI, one-shot runner, and HTTP server composition root
  desktop/             React/Vite frontend and Tauri host
  web/                 React/Vite web entrypoint and HTTP adapter
crates/
  ariadne-config/      Versioned TOML profile catalog and validation
  ariadne-core/        Domain types, model-provider port, and agent orchestration
  ariadne-provider-openai/  OpenAI-compatible HTTP adapter
  ariadne-server/      Axum API and static SPA hosting
packages/
  ui/                  Shared React conversation UI and client contract
docs/
  adr/                 Architecture decisions
```

See [the architecture guide](docs/architecture.md) for dependency boundaries and extension points.

## Prerequisites

- Rust 1.88 or newer
- Node.js 22 or newer and npm
- [Ollama](https://ollama.com/) for the default local provider, or another OpenAI-compatible endpoint
- Tauri 2 platform prerequisites when building the desktop app

## Local quick start

Start Ollama and install the default model:

```bash
ollama serve
ollama pull qwen3:8b
```

In another terminal, start an interactive session:

```bash
cargo run -p ariadne-cli -- chat
```

In the interactive terminal, type `/` to open command typeahead. Use the arrow keys to select a command, Tab to complete it, and Enter to run it. Available commands are `/clear`, `/help`, and `/quit`; `/exit` is an alias for `/quit`. Thinking-model reasoning streams into a dim section while it is active, collapses when the user-facing answer begins, and can be expanded or collapsed with Ctrl-T.

Run one unattended request:

```bash
cargo run -p ariadne-cli -- run --prompt "Summarize this repository" --output json
printf 'Draft a release checklist' | cargo run -p ariadne-cli -- run --output json
```

With a profile catalog, select a profile explicitly or list the available profiles without contacting a provider:

```bash
cargo run -p ariadne-cli -- --config ariadne.example.toml profiles
cargo run -p ariadne-cli -- --config ariadne.example.toml --profile local chat
cargo run -p ariadne-cli -- --config ariadne.example.toml --profile work run --prompt "Review this change"
```

## Web application

For frontend hot reload, run the API and Vite separately:

```bash
cargo run -p ariadne-cli -- serve
npm run dev
```

Open <http://127.0.0.1:5173>. Vite proxies API requests to port 3000. Press Enter in the composer to submit; use Shift-Enter or Alt-Enter to insert a newline. The browser streams typed thinking and content events from the server, keeps the active thinking section open, and collapses it when the user-facing answer begins. Select the Thinking summary to expand or collapse it later.

To exercise the production topology, build the SPA and serve it from the Rust process:

```bash
npm run web:build
cargo run -p ariadne-cli -- serve --web-dir apps/web/dist
```

Open <http://127.0.0.1:3000>.

## Desktop application

```bash
npm install
npm run desktop:dev
```

The desktop frontend uses narrow Tauri commands and a typed IPC channel instead of opening the HTTP server. Its shared composer submits with Enter and inserts newlines with Shift-Enter or Alt-Enter. It provides the same streaming, collapsible thinking display as the browser, loads the same profile catalog as the CLI, and shows the selected model, provider, active skills, and MCP servers.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `ARIADNE_CONFIG` | platform config path | Explicit profile-catalog path; equivalent to `--config` |
| `ARIADNE_PROFILE` | catalog `default_profile` | Process default profile; equivalent to `--profile` |
| `ARIADNE_API_BASE` | `http://127.0.0.1:11434/v1` | OpenAI-compatible API base URL |
| `ARIADNE_MODEL` | `qwen3:8b` | Provider model identifier |
| `ARIADNE_API_KEY` | unset | Optional bearer token; never place it in source control |
| `ARIADNE_SYSTEM_PROMPT` | Ariadne's built-in policy | Trusted instruction prepended by the core |
| `RUST_LOG` | `warn` | Rust tracing filter, such as `ariadne=info` |
| `VITE_ARIADNE_API_URL` | same origin | Optional API origin when an external reverse proxy supplies an appropriate CORS policy |

Copy `.env.example` as a reference, but load secrets through your shell, service manager, or secret store. Ariadne does not automatically read `.env` files. CLI flags and the legacy provider environment variables override only the selected default profile, in this order: explicit flag/environment override, selected profile, built-in local Ollama default.

When `ARIADNE_API_KEY` is set, Ariadne requires HTTPS except for loopback development endpoints (`localhost`, `127.0.0.1`, and `::1`). Unsupported URL schemes and provider URLs containing embedded credentials are rejected. Interactive terminal responses use OpenAI-compatible SSE streaming so output appears incrementally while the composer remains editable. Provider response bodies are capped at 1 MiB.

### Profile catalog

Ariadne reads TOML from the platform configuration directory at `<config-dir>/ariadne/config.toml`. On macOS this is under `~/Library/Application Support`; on Linux it normally follows `XDG_CONFIG_HOME` or `~/.config`; on Windows it uses the roaming application-data directory. If the file does not exist, Ariadne creates no files and uses the previous built-in `default` profile backed by local Ollama.

See [`ariadne.example.toml`](ariadne.example.toml) for the complete version 1 schema. The catalog separates reusable provider connections from profiles:

- `providers.<name>` defines `kind = "openai-compatible"`, `api_base`, and optional `api_key_env`. Store the secret in the named environment variable, never in TOML.
- `profiles.<name>` selects a provider and model and may define `system_prompt`, `active_skills`, and `mcp_servers`.
- `mcp_servers.<name>` stores a structured MCP server definition. Every profile reference is validated when the catalog loads.
- `default_profile` selects the profile used when a request or process does not specify one.

Profile-scoped skill and MCP activation is represented and exposed consistently now. Actual skill loading and MCP tool execution remain future capabilities and are not implied by listing an item as active.

## HTTP API

`GET /v1/profiles` returns the process default and safe profile metadata. It never returns API keys, API-key environment-variable names, provider base URLs, system prompts, or MCP command definitions.

`POST /v1/respond` accepts caller-owned user/assistant history, an optional profile name, and a new prompt. Omit `profile` to use the process default:

```json
{
  "profile": "work",
  "prompt": "Continue the investigation",
  "history": [
    { "role": "user", "content": "Inspect the logs" },
    { "role": "assistant", "content": "I found a timeout" }
  ]
}
```

The response is:

```json
{
  "message": { "role": "assistant", "content": "..." }
}
```

`POST /v1/respond/stream` accepts the same request and returns `text/event-stream`. Each data event is JSON with `kind` set to `thinking` or `content` and a `content` string. The final event has `kind: "done"` and the complete assistant `message`; failures after streaming starts use `kind: "error"` with a safe `message`.

`GET /healthz` reports process readiness. The initial API is stateless: callers send history on each request.

## VPS deployment

The server binds to `127.0.0.1:3000` by default. Keep that default and expose Ariadne through an authenticated TLS reverse proxy, VPN, or private network. Ariadne does **not** yet provide public-edge authentication, rate limiting, or load shedding, so configure those controls at the proxy for shared deployments. The built-in server is same-origin by default and does not enable CORS; configure that explicitly at a trusted reverse proxy if the web UI and API use different origins.

The Compose configuration publishes only to host loopback by default and restarts the stateless service automatically. Its default provider URL is `http://host.docker.internal:11434/v1`; Docker Desktop provides that host name, while Compose maps it through Docker's `host-gateway` on Linux.

On a Linux host, Ollama's default loopback-only listener is not reachable from a bridge-networked container. Start Ollama so it listens beyond loopback before starting Ariadne (or set the same `OLLAMA_HOST` value in the Ollama systemd service):

```bash
OLLAMA_HOST=0.0.0.0:11434 ollama serve
```

Keep TCP port `11434` firewalled from public ingress; it should be reachable only from Docker/private host networks. In another terminal, ensure the model is installed and then start Ariadne:

```bash
ollama pull qwen3:8b
docker compose up --build -d
```

Remote OpenAI-compatible providers remain supported by setting `ARIADNE_API_BASE`, `ARIADNE_MODEL`, and, when required, `ARIADNE_API_KEY` in the deployment environment. For multiple profiles, mount a catalog read-only, set `ARIADNE_CONFIG` to its in-container or server path, and supply every referenced `api_key_env` through the deployment secret store. For a native deployment, adapt [`deploy/ariadne.service`](deploy/ariadne.service).

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo audit
npm run check
npm test
npm run build
npm audit --audit-level=high
```

Install `cargo-audit` once with `cargo install cargo-audit --locked` before running the Rust dependency audit locally.

Behavior changes follow test-driven development. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Ariadne is available under the [MIT License](LICENSE).

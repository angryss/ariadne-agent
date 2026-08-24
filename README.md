# Ariadne

Ariadne is an open-source, local-first AI agent built with Rust, React, and Tauri. One shared application core powers an interactive CLI, deterministic one-shot jobs, a long-running HTTP service, a browser UI, and a native desktop app.

> **Project status:** bootstrap foundation. The model-provider path and all product surfaces are working, but tool execution, durable memory, approvals, and long-running agent loops are intentionally future capabilities.

## Why Ariadne

- **Local by default:** connects to an OpenAI-compatible endpoint at local Ollama by default.
- **Automation friendly:** `ariadne run` reads a flag or stdin and supports machine-readable JSON.
- **VPS ready:** `ariadne serve` is stateless, handles graceful shutdown, and can serve the web build from the same binary.
- **One core, several surfaces:** HTTP, terminal, and Tauri code remain thin adapters around `ariadne-core`.
- **Provider portable:** use Ollama locally or set environment variables for another OpenAI-compatible API.

## Repository layout

```text
apps/
  cli/                 Rust CLI, one-shot runner, and HTTP server composition root
  desktop/             React/Vite frontend and Tauri host
  web/                 React/Vite web entrypoint and HTTP adapter
crates/
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

Run one unattended request:

```bash
cargo run -p ariadne-cli -- run --prompt "Summarize this repository" --output json
printf 'Draft a release checklist' | cargo run -p ariadne-cli -- run --output json
```

## Web application

For frontend hot reload, run the API and Vite separately:

```bash
cargo run -p ariadne-cli -- serve
npm run dev
```

Open <http://127.0.0.1:5173>. Vite proxies API requests to port 3000.

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

The desktop frontend uses a narrow Tauri command instead of opening the HTTP server. Provider configuration still comes from the same environment variables.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `ARIADNE_API_BASE` | `http://127.0.0.1:11434/v1` | OpenAI-compatible API base URL |
| `ARIADNE_MODEL` | `qwen3:8b` | Provider model identifier |
| `ARIADNE_API_KEY` | unset | Optional bearer token; never place it in source control |
| `ARIADNE_SYSTEM_PROMPT` | Ariadne's built-in policy | Trusted instruction prepended by the core |
| `RUST_LOG` | `warn` | Rust tracing filter, such as `ariadne=info` |
| `VITE_ARIADNE_API_URL` | same origin | Optional API origin when an external reverse proxy supplies an appropriate CORS policy |

Copy `.env.example` as a reference, but load secrets through your shell, service manager, or secret store. Ariadne does not automatically read `.env` files.

When `ARIADNE_API_KEY` is set, Ariadne requires HTTPS except for loopback development endpoints (`localhost`, `127.0.0.1`, and `::1`). Unsupported URL schemes are rejected. Non-streaming provider responses are capped at 1 MiB and read incrementally.

## HTTP API

`POST /v1/respond` accepts caller-owned user/assistant history and a new prompt:

```json
{
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

Remote OpenAI-compatible providers remain supported by setting `ARIADNE_API_BASE`, `ARIADNE_MODEL`, and, when required, `ARIADNE_API_KEY` in the deployment environment. For a native deployment, adapt [`deploy/ariadne.service`](deploy/ariadne.service).

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

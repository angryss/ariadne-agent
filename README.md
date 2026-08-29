# Ariadne

Ariadne is an open-source, local-first AI agent built with Rust, React, and Tauri. One shared application core powers an interactive CLI, deterministic one-shot jobs, a long-running HTTP service, a browser UI, and a native desktop app.

> **Project status:** bootstrap foundation. The model-provider path, versioned profiles, native workspace filesystem and bounded command tools, and all product surfaces are working. Skill execution, MCP tool execution, durable memory, approvals, and long-running autonomous loops remain future capabilities.

## Why Ariadne

- **Local by default:** connects to an OpenAI-compatible endpoint at local Ollama by default.
- **Automation friendly:** `ariadne run` reads a flag or stdin and supports machine-readable JSON.
- **VPS ready:** `ariadne serve` is stateless, handles graceful shutdown, and can serve the web build from the same binary.
- **One core, several surfaces:** HTTP, terminal, and Tauri code remain thin adapters around `ariadne-core`.
- **Provider portable:** use Ollama locally or set environment variables for another OpenAI-compatible API.
- **Profile scoped:** local, work, automation, and hosted profiles can select different providers, models, system prompts, native capabilities, active skills, and MCP servers.

## Repository layout

```text
apps/
  cli/                 Rust CLI, one-shot runner, and HTTP server composition root
  desktop/             React/Vite frontend and Tauri host
  web/                 React/Vite web entrypoint and HTTP adapter
crates/
  ariadne-config/      Versioned TOML profile catalog and validation
  ariadne-core/        Domain types, model-provider port, and agent orchestration
  ariadne-provider-anthropic/ Anthropic Messages API and Claude subscription adapters
  ariadne-provider-openai/  OpenAI-compatible HTTP adapter
  ariadne-server/      Axum API and static SPA hosting
  ariadne-tools-filesystem/ Native workspace-scoped filesystem tool adapter
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
- [OpenAI Codex CLI 0.149.1](https://developers.openai.com/codex/cli/) to configure OpenAI with a ChatGPT subscription or API key; the desktop account-backed chat provider rejects unreviewed Codex versions fail-closed
- [Claude Code 2.1.223](https://docs.anthropic.com/en/docs/claude-code) for Claude subscription / usage bundle profiles
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

Open the terminal provider settings interface with:

```bash
cargo run -p ariadne-cli -- --configure-providers
```

The TUI starts with an empty provider list and supports adding, editing, and deleting Ollama, OpenAI, and Anthropic settings. Use `--provider-config <path>` to select a non-default provider settings file. OpenAI can authenticate through an API key sent directly to Codex over stdin or through Codex's ChatGPT browser sign-in.

## Web application

For frontend hot reload, run the API and Vite separately:

```bash
cargo run -p ariadne-cli -- serve
npm run dev
```

Open <http://127.0.0.1:5173>. Vite proxies API requests to port 3000. Open **Settings**, then **Providers**, to manage Ollama and OpenAI provider settings. Press Enter in the composer to submit; use Shift-Enter or Alt-Enter to insert a newline. The browser streams typed thinking and content events from the server, keeps the active thinking section open, and collapses it when the user-facing answer begins. Select the Thinking summary to expand or collapse it later.

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

The desktop frontend uses narrow Tauri commands and a typed IPC channel instead of opening the HTTP server. Its shared **Settings** → **Providers** page provides the same blank-by-default provider CRUD as the browser and CLI. Its shared composer submits with Enter and inserts newlines with Shift-Enter or Alt-Enter. It provides the same streaming, collapsible thinking display as the browser, loads the same configured profile catalog as the CLI plus the reserved `openai-account` desktop profile, and shows the selected model, provider, active skills, and MCP servers.

The desktop app also exposes **Connect OpenAI**. Choose **Use ChatGPT subscription** to complete Codex's supported browser sign-in, or enter an OpenAI API key for usage-based API billing. When adding a ChatGPT-backed OpenAI provider later, Ariadne checks the user's existing Codex account and asks whether to reuse those ChatGPT credentials or complete a new browser sign-in in Ariadne's private Codex configuration directory. Ariadne verifies reused credentials with `codex login status`, passes API keys to Codex over stdin, and never returns credentials through Tauri IPC. After connecting, select the `openai-account` profile to send prompts through that account. This account-backed profile does not receive Ariadne tools. Its ephemeral Codex thread has no execution environment; shell, image, planning, and web-search tools are disabled, any tool lifecycle item aborts the response, and the model is instructed to answer only from the supplied conversation. The provider is pinned to the reviewed `codex-cli 0.149.1` protocol/tool surface; upgrading Codex requires an Ariadne compatibility review and release.

## Configuration

| Variable | Default | Purpose |
|---|---|---|
| `ARIADNE_CONFIG` | platform config path | Explicit profile-catalog path; equivalent to `--config` |
| `ARIADNE_PROVIDER_CONFIG` | `<config-dir>/ariadne/providers.toml` | Provider settings path; equivalent to `--provider-config` |
| `ARIADNE_PROFILE` | catalog `default_profile` | Process default profile; equivalent to `--profile` |
| `ARIADNE_API_BASE` | `http://127.0.0.1:11434/v1` | OpenAI-compatible API base URL |
| `ARIADNE_MODEL` | `qwen3:8b` | Provider model identifier |
| `ARIADNE_API_KEY` | unset | Optional bearer token; never place it in source control |
| `ANTHROPIC_API_KEY` | unset | Example direct Messages API credential referenced by an Anthropic provider's `api_key_env` |
| `ARIADNE_CODEX_PATH` | `codex` on `PATH` | Codex CLI executable used by desktop OpenAI account support |
| `ARIADNE_CODEX_HOME` | `<config-dir>/ariadne/codex` | Private Codex credential/config directory owned by Ariadne desktop |
| `ARIADNE_CLAUDE_PATH` | `claude` on `PATH` | Claude Code executable used by CLI/web/desktop provider sign-in; profile execution uses `providers.<name>.claude_program` |
| `ARIADNE_SYSTEM_PROMPT` | Ariadne's built-in policy | Trusted instruction prepended by the core |
| `RUST_LOG` | `warn` | Rust tracing filter, such as `ariadne=info` |
| `VITE_ARIADNE_API_URL` | same origin | Optional API origin when an external reverse proxy supplies an appropriate CORS policy |

Copy `.env.example` as a reference, but load secrets through your shell, service manager, or secret store. Ariadne does not automatically read `.env` files. CLI flags and the legacy provider environment variables override only the selected default profile, in this order: explicit flag/environment override, selected profile, built-in local Ollama default.

When `ARIADNE_API_KEY` is set, Ariadne requires HTTPS except for loopback development endpoints (`localhost`, `127.0.0.1`, and `::1`). Unsupported URL schemes and provider URLs containing embedded credentials are rejected. Interactive terminal responses use OpenAI-compatible SSE streaming so output appears incrementally while the composer remains editable. Provider response bodies are capped at 1 MiB.

### Profile catalog

Ariadne reads TOML from the platform configuration directory at `<config-dir>/ariadne/config.toml`. On macOS this is under `~/Library/Application Support`; on Linux it normally follows `XDG_CONFIG_HOME` or `~/.config`; on Windows it uses the roaming application-data directory. If the file does not exist, Ariadne creates no files and uses the previous built-in `default` profile backed by local Ollama.

See [`ariadne.example.toml`](ariadne.example.toml) for the complete version 1 schema. The catalog separates reusable provider connections from profiles:

- `providers.<name>` may use `openai-compatible`, `anthropic-messages`, or `claude-subscription`. Direct Anthropic profiles use `api_key_env` (normally `ANTHROPIC_API_KEY`); store the secret only in that environment variable, never in TOML.
- `claude-subscription` uses Claude Code's supported headless interface after `claude auth login --claudeai` (or an explicit `CLAUDE_CODE_OAUTH_TOKEN` created by `claude setup-token`). Claude subscription / usage bundle billing is handled by Claude. Ariadne removes competing API, profile, gateway, and cloud-provider environment overrides; disables Claude Code tools, MCP, customizations, and persistence; and rejects profiles that declare Ariadne capabilities, skills, or MCP servers.
- Provider settings in the CLI, web app, and desktop app store credential readiness only. Runtime provider, model, and profile routing remains authoritative in `ariadne.toml` and is loaded at process startup.
- `profiles.<name>` selects a provider and model and may define `system_prompt`, `capabilities`, `active_skills`, and `mcp_servers`.
- `capabilities.<name>` defines an in-process native capability. `kind = "filesystem"` supplies eight workspace-scoped tools: read, write, exact edit, list, find, search, create directory, and file metadata. `kind = "command"` supplies one bounded `run_command` tool over an explicit alias-to-executable map.
- `mcp_servers.<name>` stores a structured MCP server definition. Every profile reference is validated when the catalog loads.
- `default_profile` selects the profile used when a request or process does not specify one.

Native filesystem capabilities execute today through a provider-neutral tool loop bounded to eight model turns and 64 total tool calls. Filesystem roots are explicit capability handles; path traversal is descriptor-relative and no-follow, so tool paths reject all symlinks as well as absolute and parent-traversal paths. Metadata-only operations use descriptor-relative no-follow metadata without opening file content. Content handles are opened nonblocking where the platform supports it and validated as regular files before I/O; special files are rejected or omitted from listings and traversal. Secret patterns are denied by default, and `.git` is write-protected by default. Allow globs authorize final files and visible listing results, while nonmatching policy-safe parent directories may be traversed to reach matches; `create_directory` requires its final directory path to match the allowlist. Per-file reads, result counts, total visited directory entries, traversal depth, and bytes actually read for search are bounded; writes can use bounded SHA-256 optimistic concurrency checks. `read_only`, allow/deny/protected globs, and limits are profile-catalog settings. These controls do not replace an OS sandbox for hostile or multi-tenant workloads.

Command capabilities are absent by default and currently supported only on Unix hosts. Each model-visible alias maps to one absolute executable path. At profile composition Ariadne opens authority-bearing paths nonblocking, validates the retained objects, copies at most 64 MiB from the executable handle into a private execution directory, and retains an open handle to the configured working directory. Configured-source pathname replacement after composition by actors outside Ariadne's OS identity therefore does not substitute different authorized objects. Root and every process running as Ariadne's UID are trusted and outside this application boundary: they can tamper with Ariadne's private snapshot or process state. Restart Ariadne to pick up an executable update. `run_command` starts the private executable directly rather than invoking a shell, clears the inherited environment, uses null stdin, accepts at most 128 arguments and 32 KiB of argument text, and returns structured status and UTF-8 output. Each call is capped at 300 seconds and 1 MiB of combined stdout/stderr, with configured limits allowed only at or below those hard maxima. Output work is reserved before reads and is limited to the configured byte count plus one overflow-detection byte. Every invocation runs in a new process group; on timeout, output overflow, core cancellation, or future drop, an independent supervisor sends `SIGKILL` to that group and waits for Ariadne's direct child. Orphaned descendants are reaped by the operating system. Only descendants that remain in the invocation's process group are signaled; a mapped executable can deliberately call `setpgid` or `setsid` and escape this mechanism, so only trusted executables should be mapped and hostile workloads require an OS sandbox. Cleanup failures are surfaced in the tool error and process diagnostics. The core additionally caps each response at 64 tool calls, five minutes of aggregate tool-loop time, and 8 MiB of serialized tool results.

For example, a macOS profile can map `uname` to `/usr/bin/uname` and `sw_vers` to `/usr/bin/sw_vers`, allowing prompts such as “What operating system is installed on this computer?” without granting an implicit shell. Mapping a shell, interpreter, package manager, or similarly powerful executable intentionally grants the model the authority of that program and its arguments. An executable allowlist and process group are not an OS sandbox: mapped programs retain every permission of the Ariadne process. Run Ariadne as a dedicated restricted OS user or inside a container with narrow mounts and network policy for untrusted or multi-tenant workloads.

Profile-scoped skill and MCP activation is represented and exposed consistently, but actual skill loading and MCP tool execution remain future capabilities and are not implied by listing an item as active.

## HTTP API

`GET /v1/profiles` returns the process default and safe profile metadata. It never returns API keys, API-key environment-variable names, provider base URLs, system prompts, or MCP command definitions.

`GET /v1/providers`, `POST /v1/providers`, and `PUT`/`DELETE /v1/providers/{kind}` provide provider settings CRUD for the browser. The persisted TOML contains only Ollama's API base URL or the selected OpenAI/Anthropic authentication method. OpenAI API keys are piped to Codex and are never stored in this file or returned by the API.

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

The server binds to `127.0.0.1:3000` by default. Keep that default and expose Ariadne through an authenticated TLS reverse proxy, VPN, or private network. Ariadne does **not** yet provide public-edge authentication, rate limiting, or load shedding, so configure those controls at the proxy for shared deployments. Administrative provider endpoints additionally require the direct TCP peer to be loopback; direct non-loopback requests are rejected. A same-host authenticated TLS reverse proxy can therefore administer providers, while a remotely bound Ariadne listener cannot expose those operations directly. This requirement is especially important because provider operations modify local settings and OpenAI API keys transit the authenticated request to Codex. Never expose them over unauthenticated or plaintext transport. The built-in server is same-origin by default and does not enable CORS; configure that explicitly at a trusted reverse proxy if the web UI and API use different origins.

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

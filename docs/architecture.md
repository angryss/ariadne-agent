# Ariadne architecture

Ariadne uses ports and adapters so one application core can serve interactive local users and unattended VPS workloads.

```text
React UI package <--- web fetch adapter ---> Axum server ---+
       ^                                                   |
       +-------- Tauri invoke adapter ---> desktop host ---+---> application core ---> model provider port
                                                           |
CLI interactive / run / serve ----------------------------+
                 ^
                 +--- versioned profile catalog ---> provider/model composition
```

## Boundaries

1. **Core** owns messages, requests, provider ports, profile metadata, profile dispatch, and agent orchestration. It performs no network, terminal, web, desktop, or configuration-file I/O.
2. **Configuration** parses and validates the versioned TOML provider/profile catalog. It resolves safe profile metadata and provider inputs but never reads provider credentials itself.
3. **Adapters** implement model-provider and transport concerns. The initial provider targets OpenAI-compatible APIs, including local Ollama.
4. **Composition roots** choose concrete adapters for CLI, HTTP server, and Tauri desktop execution. They read the environment variables named by providers and apply legacy CLI/environment overrides to the selected default profile.
5. **UI** depends on a small TypeScript client port. The web app implements it with HTTP; the desktop app implements it with Tauri IPC. Both fetch safe profile metadata and clear caller-owned history when a user switches profiles.

## Operating modes

- `ariadne`: local interactive terminal session.
- `ariadne run`: deterministic one-shot process suitable for scripts, cron, and systemd.
- `ariadne serve`: long-lived HTTP and web process suitable for a VPS or local browser.
- Ariadne Desktop: native shell using the same core through narrow Tauri commands.

The server is stateless in the initial bootstrap. Callers provide conversation history with each request, which keeps horizontal scaling possible and defers persistence policy to a later capability.

A server or desktop process composes every catalog profile into an `AgentProfiles` registry. Each request can select one profile; an omitted profile uses the process default. CLI chat and one-shot modes select one default profile through `--profile` or `ARIADNE_PROFILE`.

## Security posture

The server binds to `127.0.0.1` by default. Operators exposing it publicly must put it behind an authenticated TLS reverse proxy or private network. Provider credentials are accepted through environment variables and are never returned by APIs or stored by the frontend. The profiles endpoint returns only names, provider aliases, models, and capability activation names; it omits provider URLs, credential-variable names, system prompts, and MCP command definitions.

## Extension points

Model providers, skills, MCP tools, memory, session persistence, and approval policies should enter through explicit ports. Profile-scoped skill and MCP activation is already represented by safe names, but loading and execution remain future capabilities. New surfaces should compose those ports rather than duplicate orchestration.

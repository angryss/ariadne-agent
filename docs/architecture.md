# Ariadne architecture

Ariadne uses ports and adapters so one application core can serve interactive local users and unattended VPS workloads.

```text
React UI package <--- web fetch adapter ---> Axum server ---+
       ^                                                   |
       +-------- Tauri invoke adapter ---> desktop host ---+---> application core ---> model provider port
                                                          |
CLI interactive / run / serve ----------------------------+
```

## Boundaries

1. **Core** owns messages, requests, provider ports, and agent orchestration. It performs no network, terminal, web, or desktop I/O.
2. **Adapters** implement model-provider and transport concerns. The initial provider targets OpenAI-compatible APIs, including local Ollama.
3. **Composition roots** choose concrete adapters for CLI, HTTP server, and Tauri desktop execution.
4. **UI** depends on a small TypeScript client port. The web app implements it with HTTP; the desktop app implements it with Tauri IPC.

## Operating modes

- `ariadne`: local interactive terminal session.
- `ariadne run`: deterministic one-shot process suitable for scripts, cron, and systemd.
- `ariadne serve`: long-lived HTTP and web process suitable for a VPS or local browser.
- Ariadne Desktop: native shell using the same core through narrow Tauri commands.

The server is stateless in the initial bootstrap. Callers provide conversation history with each request, which keeps horizontal scaling possible and defers persistence policy to a later capability.

## Security posture

The server binds to `127.0.0.1` by default. Operators exposing it publicly must put it behind an authenticated TLS reverse proxy or private network. Provider credentials are accepted through environment variables and are never returned by APIs or stored by the frontend.

## Extension points

Model providers, tools, memory, session persistence, and approval policies should enter through explicit ports. New surfaces should compose those ports rather than duplicate orchestration.

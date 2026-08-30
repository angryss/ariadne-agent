# Rynna architecture

Rynna uses ports and adapters so one application core can serve interactive local users and unattended VPS workloads.

```text
React UI package <--- web fetch adapter ---> Axum server ---+
       ^                                                   |
       +-------- Tauri invoke adapter ---> desktop host ---+---> application core ---> model provider port
                                                           |             |
                                                           |             +---> tool port ---> native filesystem/command adapters
                                                           |
CLI interactive / run / serve ----------------------------+
                 ^
                 +--- versioned profile catalog ---> provider/model composition
```

## Boundaries

1. **Core** owns messages, requests, provider ports, cache-optimization policy, profile metadata, profile dispatch, and agent orchestration. It performs no network, terminal, web, desktop, or configuration-file I/O. `CacheOptimizer` is a replaceable strategy: the built-in prefix optimizer derives a stable, non-secret scope from system instructions, the first conversation message, and ordered tool definitions, while provider adapters translate that optimization into technology-specific server-cache controls. The conversation anchor keeps one routing scope stable as history grows and separates differing initial prefixes; byte-identical prefixes share a scope because stateless requests do not carry a conversation identifier.
2. **Configuration** parses and validates the versioned TOML provider/profile catalog. It resolves safe profile metadata, provider inputs, and native capability settings but never reads provider credentials itself.
3. **Adapters** implement model-provider, transport, cache-control translation, and tool concerns. Providers target OpenAI-compatible APIs, Anthropic's Messages API (SSE plus provider-neutral tool translation), and Claude Code's supported headless account interface for Claude subscription / usage bundle access. Official OpenAI requests receive a stable `prompt_cache_key`; Anthropic Messages requests enable automatic ephemeral prompt caching; Ollama keeps its ordered OpenAI-compatible prompt shape so Ollama's automatic prefix cache can reuse it without receiving unsupported OpenAI-only fields. Claude Code manages the subscription path's server caching. The account-backed path disables internal tools and rejects Rynna tools. Native adapters implement the core tool port for a canonical workspace filesystem and for bounded, explicitly mapped host programs.
4. **Composition roots** choose concrete adapters for CLI, HTTP server, and Tauri desktop execution. They read the environment variables named by providers and apply legacy CLI/environment overrides to the selected default profile.
5. **UI** depends on a small TypeScript client port. The web app implements it with HTTP; the desktop app implements it with Tauri IPC. Both fetch safe profile metadata and clear caller-owned history when a user switches profiles.

## Operating modes

- `rynna`: local interactive terminal session.
- `rynna run`: deterministic one-shot process suitable for scripts, cron, and systemd.
- `rynna serve`: long-lived HTTP and web process suitable for a VPS or local browser.
- Rynna Desktop: native shell using the same core through narrow Tauri commands.

The server is stateless in the initial bootstrap. Callers provide conversation history with each request, which keeps horizontal scaling possible and defers persistence policy to a later capability. Tool calls and tool results exist only inside one response run and are not accepted in caller-owned history.

A server or desktop process composes every catalog profile into an `AgentProfiles` registry. Each request can select one profile; an omitted profile uses the process default. CLI chat and one-shot modes select one default profile through `--profile` or `RYNNA_PROFILE`.

## Security posture

The server binds to `127.0.0.1` by default. Operators exposing it publicly must put it behind an authenticated TLS reverse proxy or private network. Provider credentials are accepted through environment variables and are never returned by APIs or stored by the frontend. The profiles endpoint returns only names, provider aliases, models, and activation names; it omits provider URLs, credential-variable names, system prompts, native capability details, and MCP command definitions.

The filesystem adapter opens each configured root as a capability directory and traverses every tool-path component descriptor-relatively with no-follow semantics; all symlinks in tool paths are rejected. Metadata-only operations use descriptor-relative no-follow metadata. Content opens are nonblocking where supported and the opened handle must be a regular file before I/O, so listings and traversal skip special files and direct content operations reject them. Allow globs apply to final files and visible listing entries, not policy-safe ancestor directories needed to reach them; deny policy remains active during traversal, protected policy remains active for writes, and `create_directory` requires its final path to match the allowlist. The adapter also rejects absolute paths, parent traversal, denied paths, protected writes, stale expected hashes, and configured per-file, result, total-visited-entry, traversal-depth, and actual-search-read-byte overruns. Search/traversal stop immediately when result or byte limits are exhausted. Container mounts, dedicated OS users, and read-only or otherwise restricted host filesystems remain the deployment security boundary for hostile workloads.

The command adapter contributes one `run_command` tool only when a profile activates a command capability. Model-visible aliases map to configured absolute executable paths whose validated bytes are snapshotted at profile composition. Execution does not pass through a shell, inherits no environment, receives no stdin, runs in one configured working directory, and is bounded by argument, timeout, and combined-output limits. These controls prevent accidental ambient shell and secret-environment access but do not constrain what an intentionally mapped program can do. Low-privilege OS identities, containers, mounts, and OS sandboxing remain the security boundary.

## Extension points

Model providers and native tools enter through explicit core ports. The bounded agent loop sends provider-neutral tool definitions, executes at most 64 requested tools across at most seven tool-producing turns, returns structured results to the model, and reserves the eighth turn for a final answer. Streaming forwards thinking deltas immediately, while buffering user-facing content until the turn is known to be final so intermediate tool-turn content is not exposed as answer text. Skills, MCP tools, memory, session persistence, and approval policies should follow the same boundary discipline. Profile-scoped skill and MCP activation is represented by safe names, but loading and execution remain future capabilities. New surfaces should compose those ports rather than duplicate orchestration.

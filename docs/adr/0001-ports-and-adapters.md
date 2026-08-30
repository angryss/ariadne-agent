# ADR 0001: Share a ports-and-adapters core across all surfaces

- Status: Accepted
- Date: 2026-08-24

## Context

Rynna must support an interactive CLI, browser UI, Tauri desktop app, one-shot automation, and a long-running VPS process without allowing their lifecycle and transport concerns to fork agent behavior.

## Decision

Use a Rust application core with provider ports. Keep HTTP, terminal, Tauri, and provider SDK details in adapters. Use a platform-neutral React UI package with separate HTTP and Tauri client adapters.

The initial HTTP API is stateless. Conversation history is supplied by the caller. The CLI executable also owns the server subcommand so VPS deployment requires one binary and optional static web assets.

## Consequences

- Agent behavior is reusable and testable without network or UI processes.
- All product surfaces can evolve independently around stable ports.
- Adapter wiring is slightly more verbose.
- Durable sessions, tool execution, and memory require explicit future ports rather than direct framework calls.

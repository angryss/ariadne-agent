# Contributing to Rynna

Thank you for improving Rynna.

## Design rules

- Keep domain and orchestration behavior in `rynna-core`.
- Keep provider, HTTP, CLI, Tauri, and browser concerns in adapters.
- Depend inward; the core must not import a transport or UI framework.
- Preserve non-interactive behavior: JSON mode emits one JSON value to stdout, diagnostics go to stderr, and failures return a non-zero status.
- Do not log prompts, authorization headers, API keys, or other secrets at info level.
- Keep the server on loopback by default.

## Test-driven workflow

1. Add one focused failing test for the desired behavior.
2. Run it and confirm that it fails for the expected reason.
3. Add the smallest implementation that passes.
4. Run the focused test and then the complete relevant suite.
5. Refactor only while tests stay green.

## Quality gate

Run before opening a pull request:

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

Install `cargo-audit` once with `cargo install cargo-audit --locked` if it is not already available.

Changes to the Tauri host should also pass:

```bash
cargo check -p rynna-desktop
```

## Pull requests

Keep pull requests focused, explain architectural consequences, and include tests for behavior changes. Never commit `.env` files, provider keys, model credentials, signing keys, or generated build directories.

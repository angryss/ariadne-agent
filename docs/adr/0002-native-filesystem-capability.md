# 0002. Native workspace filesystem capability

- Status: accepted
- Date: 2026-08-25

## Context

Ariadne needs local filesystem access in CLI, HTTP, web, and desktop modes. Skills describe workflows but do not grant authority. MCP can supply tools, but making an external filesystem server the only implementation would add process and transport dependencies to a foundational local capability. Filesystem access also needs one consistently enforced boundary for traversal, symlinks, secrets, writes, limits, and unattended operation.

## Decision

Ariadne exposes provider-neutral tool definitions and tool calls from `ariadne-core`. The agent runs a bounded loop of at most eight model turns and 64 total tool calls, executes tools through a `Tool` port only when a follow-up model turn remains, and returns structured tool results to the provider. Caller-owned history remains restricted to plain user and assistant messages without internal tool metadata. Streaming buffers each turn until it is known to be final and suppresses intermediate tool-turn content.

The first implementation is an in-process `ariadne-tools-filesystem` adapter activated by profile-scoped native capability names. One filesystem capability contributes read, write, exact edit, list, find, search, create-directory, and file-info tools under a canonical root.

The adapter rejects absolute and parent-traversal paths and rejects every symlink in a tool path. It traverses path components from open directory handles with descriptor-relative no-follow operations, including final files, directory listing/traversal, and component-by-component directory creation. Metadata-only operations use descriptor-relative no-follow metadata without opening content. Content opens request nonblocking behavior where supported and validate the opened handle as a regular file before any read or write, closing metadata-to-open races that could otherwise block on a FIFO or other special file; direct content and file-info requests reject special files, while listing and traversal omit them. Denied globs apply to final paths and traversed components. Protected globs apply to final write targets and write traversal. Allowed globs authorize final read/write/file-info targets and visible listing or traversal results, but do not reject nonmatching policy-safe ancestor directories needed to reach a matching target; `list_directory` therefore permits traversal to a policy-safe directory and filters its returned entries, while `create_directory` requires the final requested directory path itself to match the allowlist. Common secret files are denied by default, and `.git` writes are protected by default. The adapter bounds per-file reads, result counts, total visited directory entries, traversal depth, bytes actually read for search, and hash verification work; stops traversal immediately on result or search-byte exhaustion; detects binary reads; and supports SHA-256 optimistic concurrency for writes and edits. Deployments must still use OS or container filesystem isolation when the model or tenant is untrusted.

Skills remain instructions layered over available tools. MCP remains a future alternate adapter that can contribute tools through the same core port rather than a special filesystem abstraction.

## Consequences

- Every product surface gets identical filesystem behavior through the shared core and profile composition.
- Filesystem authority is explicit in the profile catalog and absent by default.
- OpenAI-compatible non-streaming and streaming tool calls require translation in the provider adapter.
- The application-level boundary is testable and reusable but is not a replacement for mounts, dedicated users, containers, or other OS sandboxing.
- Approval policy is still a future port; operators should prefer read-only profiles until it exists.

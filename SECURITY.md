# Security policy

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting for this repository. Do not disclose exploitable details in a public issue.

Include the affected version, reproduction steps, impact, and any suggested mitigation. Maintainers will acknowledge a complete report as soon as practical and coordinate disclosure after a fix is available.

## Deployment boundary

Ariadne's HTTP service is not currently a public security boundary. It binds to loopback by default and should be exposed only through an authenticated TLS reverse proxy, VPN, or private network. Treat model-provider credentials and conversation contents as sensitive.

Profile catalogs may contain provider endpoints and MCP command definitions, so mount them read-only and do not expose them as static web assets. Store credentials only in the environment variables named by `api_key_env`; URL-embedded credentials are rejected. The `/v1/profiles` response intentionally omits provider URLs, credential-variable names, system prompts, and MCP command definitions.

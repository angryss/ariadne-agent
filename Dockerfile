FROM node:26-bookworm-slim AS web-builder
WORKDIR /workspace
COPY package.json package-lock.json tsconfig.base.json ./
COPY apps/web/package.json apps/web/package.json
COPY apps/desktop/package.json apps/desktop/package.json
COPY packages/ui/package.json packages/ui/package.json
RUN npm ci
COPY apps/web apps/web
COPY packages/ui packages/ui
RUN npm run web:build

FROM rust:1.96-bookworm AS rust-builder
WORKDIR /workspace
COPY . .
RUN cargo build --locked --release -p ariadne-cli

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 ariadne
COPY --from=rust-builder /workspace/target/release/ariadne /usr/local/bin/ariadne
COPY --from=web-builder /workspace/apps/web/dist /opt/ariadne/web
USER ariadne
EXPOSE 3000
ENV ARIADNE_API_BASE=http://host.docker.internal:11434/v1 \
    ARIADNE_MODEL=qwen3:8b \
    RUST_LOG=warn
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:3000/healthz >/dev/null || exit 1
ENTRYPOINT ["ariadne"]
CMD ["serve", "--bind", "0.0.0.0:3000", "--web-dir", "/opt/ariadne/web"]

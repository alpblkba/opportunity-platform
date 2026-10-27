# Production image for the API, the scheduler and the built frontend, which are one process by design.
# Multi-stage so the shipped image carries no toolchain, and multi-arch because the target VPS may be amd64 or arm64.
#
# Two runtime targets:
#   --target runtime  (default) API, scheduler and interface. No browser: applications are tracked and every submission is a manual handoff.
#   --target browser            the same, plus Node and Chromium so the browser worker can inspect and fill forms.
# The worker is still a separate process spawned per task and driven over stdin; putting it in the same image only gives that process a Node to run in. Nothing is exposed over a network to reach it, and a Chromium crash still cannot take the API down.

FROM --platform=$BUILDPLATFORM node:22-bookworm-slim AS web
WORKDIR /build
COPY apps/web/package.json apps/web/package-lock.json ./apps/web/
RUN cd apps/web && npm ci
COPY apps/web ./apps/web
RUN cd apps/web && npm run build

FROM --platform=$BUILDPLATFORM node:22-bookworm-slim AS worker
WORKDIR /build
COPY apps/browser-worker/package.json apps/browser-worker/package-lock.json ./
RUN npm ci
COPY apps/browser-worker ./
RUN npm run build

FROM rust:1-bookworm AS server
WORKDIR /build
# Dependencies are fetched against the manifests alone so a source edit does not invalidate the whole dependency layer.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/domain/Cargo.toml crates/domain/
COPY crates/adapters/Cargo.toml crates/adapters/
COPY crates/core/Cargo.toml crates/core/
COPY crates/api/Cargo.toml crates/api/
COPY crates/cli/Cargo.toml crates/cli/
RUN mkdir -p crates/domain/src crates/adapters/src crates/core/src crates/api/src crates/cli/src \
    && echo "fn main() {}" > crates/cli/src/main.rs \
    && for c in domain adapters core api; do echo "" > crates/$c/src/lib.rs; done \
    && cargo fetch --locked
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --locked --release --bin opportunity && strip target/release/opportunity

FROM debian:bookworm-slim AS runtime
ARG GIT_REVISION
LABEL org.opencontainers.image.revision=$GIT_REVISION \
      org.opencontainers.image.title="opportunity-runtime"
# ca-certificates is needed to read job boards over TLS; curl is the health check; poppler-utils provides pdftotext for PDF CVs.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl poppler-utils \
    && rm -rf /var/lib/apt/lists/*
# An unprivileged user owns the private tree, which holds the CV and browser artefacts.
RUN useradd --system --create-home --uid 10001 opportunity
WORKDIR /app
COPY --from=server /build/target/release/opportunity /usr/local/bin/opportunity
COPY --from=web /build/apps/web/dist /app/apps/web/dist
COPY config /app/config
RUN mkdir -p /app/private && chown -R opportunity:opportunity /app/private
USER opportunity
ENV WEB_DIST=/app/apps/web/dist \
    RAW_SNAPSHOT_DIR=/app/private/cache \
    API_HOST=0.0.0.0 \
    API_PORT=8787
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8787/health || exit 1
ENTRYPOINT ["opportunity"]
CMD ["serve"]

# The browser-capable runtime.
#
# The browser is installed by the same Playwright package that will drive it, rather than taken from a prebuilt browser image. Playwright pins an exact browser build per release, so a base image tagged for one version and a lockfile resolving to another produce an image with a browser in it and no browser it can use: `Executable doesn't exist at /ms-playwright/chromium_headless_shell-1248`. Installing from the resolved package makes that mismatch impossible, and brings only Chromium instead of Chromium, Firefox and WebKit.
FROM node:22-bookworm-slim AS browser
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl poppler-utils \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --create-home --uid 10001 opportunity
WORKDIR /app
COPY --from=server /build/target/release/opportunity /usr/local/bin/opportunity
COPY --from=web /build/apps/web/dist /app/apps/web/dist
COPY --from=worker /build/dist /app/browser-worker/dist
COPY --from=worker /build/node_modules /app/browser-worker/node_modules
COPY --from=worker /build/mock /app/browser-worker/mock
COPY config /app/config
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
RUN cd /app/browser-worker && npx --no-install playwright install --with-deps chromium \
    && rm -rf /var/lib/apt/lists/* && chmod -R a+rx /ms-playwright
RUN mkdir -p /app/private/browser && chown -R opportunity:opportunity /app/private
USER opportunity
ENV WEB_DIST=/app/apps/web/dist \
    RAW_SNAPSHOT_DIR=/app/private/cache \
    BROWSER_WORKER_ENTRY=/app/browser-worker/dist/cli.js \
    BROWSER_WORKER_ARTEFACTS=/app/private/browser \
    PLAYWRIGHT_BROWSERS_PATH=/ms-playwright \
    API_HOST=0.0.0.0 \
    API_PORT=8787
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8787/health || exit 1
ENTRYPOINT ["opportunity"]
CMD ["serve"]

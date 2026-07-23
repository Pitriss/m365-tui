# Build the webhook binary against the whole workspace, then ship a slim image.
FROM rust:1-slim AS build
WORKDIR /src

# Copy the full workspace (the webhook depends on m365-core).
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates

# Build only the webhook binary in release mode.
RUN cargo build --release -p webhook

FROM debian:bookworm-slim AS runtime
# ca-certificates only needed if the service ever makes outbound TLS calls; the
# webhook itself does not, but keep it for parity/health probes.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/m365-webhook /usr/local/bin/m365-webhook

ENV WEBHOOK_BIND=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/m365-webhook"]

# The webhook image, built from the prebuilt static binary shipped alongside this
# file in the release bundle. No Rust toolchain, no source tree, no compile —
# `docker compose build` here takes about a second.
#
# `m365-webhook` from the release is statically linked against musl, so Alpine
# (or even scratch) can run it as-is. A shell is kept for `docker compose exec`
# and the healthcheck's wget.
FROM alpine:3.21

# The webhook holds no credentials and never calls Graph, but there's no reason
# for it to run as root either.
RUN adduser -D -u 10001 webhook

COPY m365-webhook /usr/local/bin/m365-webhook

USER webhook
ENV WEBHOOK_BIND=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/m365-webhook"]

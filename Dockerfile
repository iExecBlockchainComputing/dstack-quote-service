FROM rust:1.93.0-alpine3.23 AS builder

WORKDIR /app

# Copy manifest and source files
COPY . .

# Build the application
RUN cargo build --release

FROM alpine:3.23 AS runtime

# Set working directory
WORKDIR /app

# Copy the binary from builder stage
COPY --from=builder /app/target/release/dstack-quote-service .

# Readiness gate: the Fluent Bit fragment, when generated, is written before the
# listener is bound, so a healthy container also means the fragment is on disk.
HEALTHCHECK --interval=5s --timeout=3s --start-period=2s --retries=12 \
    CMD wget -q -O /dev/null "http://127.0.0.1:${QUOTE_SIDECAR_SERVER__PORT:-9999}/health"

# Run the application
ENTRYPOINT ["/app/dstack-quote-service"]

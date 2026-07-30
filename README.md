# Dstack Quote Sidecar

A lightweight HTTP sidecar service for interacting with [Dstack TEE](https://github.com/Dstack-TEE/dstack) to generate attestation quotes and perform attestation operations.

## Overview

This service provides a REST API interface to the Dstack SDK, enabling easy integration with Trusted Execution Environment (TEE) attestation capabilities. It allows applications to generate cryptographic quotes and attestations for secure computation verification.

## Features

- 🔐 **Quote Generation**: Generate TEE quotes with custom data
- ✅ **Attestation**: Create attestation proofs for application state
- 📊 **RTMR Replay**: Automatic replay of Runtime Measurement Registers from event logs
- 🏷️ **Fluent Bit Fragment**: Optionally persist the CVM identity at startup for log labelling
- 🚀 **Fast & Lightweight**: Built with Axum for high-performance async operations
- 📝 **JSON API**: Simple REST endpoints with JSON responses
- 🔍 **Health Checks**: Built-in health monitoring endpoints

## Tech Stack

- **Rust**
- **Axum** - Web framework
- **Dstack SDK** - TEE attestation library
- **Tokio** - Async runtime
- **Serde** - JSON serialization

## Configuration

The service can be configured using environment variables. The naming scheme is
`QUOTE_SIDECAR_` + section + `__` + key: a **single** underscore after the prefix, and a
**double** underscore between the section and the key.

| Variable                                     | Description                                       | Default               |
|----------------------------------------------|---------------------------------------------------|-----------------------|
| `QUOTE_SIDECAR_SERVER__HOST`                 | Server bind address                                | `0.0.0.0`             |
| `QUOTE_SIDECAR_SERVER__PORT`                 | Server port                                        | `9999`                |
| `QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__GENERATE`| Write the Fluent Bit fragment at startup           | `false`               |
| `QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__PATH`    | Destination of the fragment                        | `/shared/instance.conf`|
| `QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__RETRIES` | Extra attempts to reach the guest agent            | `5`                   |
| `QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__RETRY_DELAY_MS`| Delay between two attempts, in milliseconds  | `2000`                |

### Example

```bash
export QUOTE_SIDECAR_SERVER__HOST=127.0.0.1
export QUOTE_SIDECAR_SERVER__PORT=8080
```

## Usage

### Running the Service

```bash
# Development mode
cargo run

# Production mode (release build)
cargo run --release
```

The server will start on `http://0.0.0.0:9999` by default.

## API Endpoints

### 1. Root Endpoint

**`GET /`**

Returns service information and current timestamp.

**Response:**

```json
{
  "service": "dstack-quote-service",
  "timestamp": "2026-02-12T09:30:45.123456Z"
}
```

### 2. Health Check

**`GET /health`**

Health check endpoint for monitoring.

**Response:**

```json
{
  "status": "ok"
}
```

### 3. Generate Quote

**`GET /quote`**

Generates a TEE quote for the provided data and replays RTMRs from the event log.

**Query Parameters:**

- `data` (optional): Custom data to include in the quote. Defaults to `"hello world"` if not provided.

**Examples:**

```bash
# With default data
curl http://localhost:9999/quote

# With custom data
curl "http://localhost:9999/quote?data=user:alice:nonce123"
```

**Success Response:**

```json
{
  "quote": "0x...",
  "event_log": "[{...}]",
  "vm_config": "{...}",
  "rtmrs": "Rtmrs { ... }"
}
```

**Error Response:**

```json
{
  "error": "Failed to get quote: ..."
}
```

### 4. CVM Info

**`GET /info`**

Returns the full `Info` response from the dstack guest agent: app ID, instance ID, app name,
TCB info, measurements and compose hash.

```bash
curl -s http://localhost:9999/info | jq -r .instance_id
```

**Error Response:**

```json
{
  "error": "failed to get info: ..."
}
```

### 5. Generate Attestation

**`GET /attest`**

Generates an attestation quote for the provided application state.

**Query Parameters:**

- `data` (optional): Custom data to include in the attestation. Defaults to `"hello world"` if not provided.

**Examples:**

```bash
# With default data
curl http://localhost:9999/attest

# With custom data
curl "http://localhost:9999/attest?data=my-app-state"
```

**Success Response:**

```json
{
  "attestation": "eyJ0eXAiOiJKV1QiLCJhbGc..."
}
```

**Error Response:**

```json
{
  "error": "Failed to attest: ..."
}
```

## Project Structure

```text
dstack-quote-sidecar/
├── src/
│   ├── main.rs           # Application entry point
│   ├── application.rs    # Application setup and routing
│   ├── config.rs         # Configuration management
│   ├── handlers.rs       # HTTP request handlers
│   └── fluent_bit_fragment.rs  # Startup persistence of the CVM identity
├── Cargo.toml            # Project dependencies
└── README.md             # This file
```

## Fluent Bit Fragment

Fluent Bit, running alongside this service inside the CVM, needs the dstack `instance_id` to
label the records it forwards. Rather than giving it its own access to the dstack socket (which
usually means an extra `curl` + `jq` init container), this service can persist the CVM identity
once at startup.

Turn it on with `QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__GENERATE=true`. The service queries the
guest agent and writes a Fluent Bit configuration fragment:

```ini
@SET INSTANCE_ID=...
@SET APP_ID=...
@SET APP_NAME=...
@SET COMPOSE_HASH=...
```

Fluent Bit pulls it in with an `@INCLUDE` and the values become usable as `${INSTANCE_ID}` and
friends anywhere in its configuration. `@SET` is used rather than an `.env` file because the
official Fluent Bit images are **distroless** — no `sh`, no `bash`, no `busybox` — so overriding
their entrypoint to `source` a file is not possible.

Properties worth knowing:

- **Written before the listener is bound.** A healthy container therefore also means the fragment
  is on disk, so consumers can simply wait on `condition: service_healthy`. The image ships a
  `HEALTHCHECK` that polls `/health`.
- **Atomic.** The fragment is written to a temporary path and renamed, so a reader never sees a
  partial write.
- **Retried.** The guest agent is queried once, then up to `RETRIES` more times, spaced by
  `RETRY_DELAY_MS`.
- **Fail-fast.** If the fragment cannot be written, startup aborts with a non-zero exit code.
  That is what keeps the guarantee above meaningful, and it costs nothing: the dstack socket is
  this service's only external dependency, so a guest agent that cannot be reached leaves
  `/quote`, `/attest` and `/info` broken anyway. There is deliberately no opt-out.
- Values are written bare, because `@SET` takes everything up to the end of the line literally.
  Empty values and values containing a newline are rejected at write time. Fluent Bit would
  reject them too, but only at its own startup, in another container, with an error that does
  not name the guest agent.
- `app_cert` and `tcb_info` are deliberately **not** exported. Use `GET /info` for the full payload.

### Fluent Bit integration

No entrypoint override, no extra container: mount the shared volume and include the fragment.

```yaml
services:
  dstack-quote-service:
    image: docker-regis.iex.ec/dstack-quote-service:<tag>
    environment:
      QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__GENERATE: "true"
      QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__PATH: /shared/instance.conf
    volumes:
      - /var/run/dstack.sock:/var/run/dstack.sock
      - shared:/shared

  fluent-bit:
    image: fluent/fluent-bit:<tag>
    # Required: Fluent Bit exits on a missing @INCLUDE and Docker will not
    # restart it on its own. See the note below.
    restart: unless-stopped
    depends_on:
      dstack-quote-service:
        condition: service_healthy
    volumes:
      - shared:/shared
      - ./fluent-bit.conf:/fluent-bit/etc/fluent-bit.conf

volumes:
  shared:
```

`fluent-bit.conf` includes the fragment first, then uses the values anywhere:

```ini
@INCLUDE /shared/instance.conf

[FILTER]
    Name          record_modifier
    Match         *
    Record        instance_id ${INSTANCE_ID}
    Record        app_name    ${APP_NAME}
```

Fluent Bit refuses to start if the `@INCLUDE` target is missing, which is the behaviour you want:
it can never ship unlabelled records. At `compose up`, `condition: service_healthy` already
guarantees the fragment is there. Outside of that — a host reboot, where the daemon brings
containers back in its own order — Fluent Bit may well start first and exit.

**`restart: unless-stopped` on the Fluent Bit service is therefore not optional.** Docker does
not restart a container that exits unless a restart policy says so; without one, a reboot in the
wrong order leaves Fluent Bit permanently down. With one, it retries until the fragment appears.

> **If the configuration lives in a compose `configs: content:` block, escape the variable as
> `$${INSTANCE_ID}`.** Compose interpolates `${...}` in that block at `up` time, when the value
> does not exist yet, and substitutes an empty string without warning. Writing `$$` makes Compose
> emit a literal `${INSTANCE_ID}` for Fluent Bit to resolve itself. Variables that Compose *should*
> resolve, such as a Loki hostname coming from your `.env`, keep a single `$`.

## Development with Simulator

For local development without TDX hardware, use the Dstack simulator:

### 1. Clone and Build the Simulator

```bash
git clone https://github.com/Dstack-TEE/dstack.git
cd dstack/sdk/simulator
./build.sh
```

### 2. Configure the Simulator

**Important:** The simulator needs to expose the internal API on HTTP instead of Unix sockets. Edit `dstack.toml`:

```toml
[internal]
address = "0.0.0.0:8090"
reuse = true
```

### 3. Start the Simulator

```bash
./dstack-simulator
```

The simulator will now listen on `http://0.0.0.0:8090`.

### 4. Run the Sidecar

In a separate terminal:

```bash
cd /path/to/dstack-quote-sidecar
export DSTACK_SIMULATOR_ENDPOINT=http://localhost:8090
cargo run
```

To exercise the Fluent Bit fragment as well:

```bash
export QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__GENERATE=true
export QUOTE_SIDECAR_FLUENT_BIT_FRAGMENT__PATH=/tmp/dstack-test/instance.conf
cargo run
```

`Fluent Bit fragment written to ...` is logged before `Server bound to ...`.

### 5. Test the Endpoints

```bash
# Test quote endpoint
curl "http://localhost:9999/quote?data=test123"

# Test attestation endpoint
curl "http://localhost:9999/attest?data=my-app-state"

# Test info endpoint
curl -s http://localhost:9999/info | jq -r .instance_id

# Check the Fluent Bit fragment
cat /tmp/dstack-test/instance.conf
```

## Development

### Run Tests

```bash
cargo test
```

### Check Code

```bash
cargo check
```

### Format Code

```bash
cargo fmt
```

### Lint

```bash
cargo clippy
```

## Logging

The service uses `tracing` for structured logging. Set the `RUST_LOG` environment variable to control log levels:

```bash
# Debug level
RUST_LOG=debug cargo run

# Info level (default)
RUST_LOG=info cargo run

# Trace level (verbose)
RUST_LOG=trace cargo run
```

## Graceful Shutdown

The service handles graceful shutdown on:

- `CTRL+C` (SIGINT)
- `SIGTERM` (Unix-like systems)

## License

MIT License - See LICENSE file for details

## Related Projects

- [Dstack TEE](https://github.com/Dstack-TEE/dstack) - The underlying TEE attestation framework

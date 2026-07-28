# Dstack Quote Sidecar

A lightweight HTTP sidecar service for interacting with [Dstack TEE](https://github.com/Dstack-TEE/dstack) to generate attestation quotes and perform attestation operations.

## Overview

This service provides a REST API interface to the Dstack SDK, enabling easy integration with Trusted Execution Environment (TEE) attestation capabilities. It allows applications to generate cryptographic quotes and attestations for secure computation verification.

## Features

- 🔐 **Quote Generation**: Generate TEE quotes with custom data
- ✅ **Attestation**: Create attestation proofs for application state
- 📊 **RTMR Replay**: Automatic replay of Runtime Measurement Registers from event logs
- 🏷️ **Instance File**: Optionally persist the CVM identity at startup for sidecars to consume
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
| `QUOTE_SIDECAR_INSTANCE_FILE__ENABLED`       | Write the instance file at startup                 | `false`               |
| `QUOTE_SIDECAR_INSTANCE_FILE__PATH`          | Destination of the instance file                   | `/shared/instance.env`|
| `QUOTE_SIDECAR_INSTANCE_FILE__REQUIRED`      | Abort startup if the instance file cannot be written | `true`              |
| `QUOTE_SIDECAR_INSTANCE_FILE__RETRIES`       | Extra attempts to reach the guest agent            | `5`                   |
| `QUOTE_SIDECAR_INSTANCE_FILE__RETRY_DELAY_MS`| Delay between two attempts, in milliseconds        | `2000`                |

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
│   └── instance_file.rs  # Startup persistence of the CVM identity
├── Cargo.toml            # Project dependencies
└── README.md             # This file
```

## Instance File

Log shippers running alongside this service inside the CVM — Fluent Bit, typically — need the
dstack `instance_id` to label the records they forward. Rather than giving each of them its own
access to the dstack socket (which usually means an extra `curl` + `jq` init container), this
service can persist the CVM identity once at startup.

Enable it with `QUOTE_SIDECAR_INSTANCE_FILE__ENABLED=true`. The service then queries the guest
agent and writes a shell-sourceable file:

```sh
INSTANCE_ID='...'
APP_ID='...'
APP_NAME='...'
COMPOSE_HASH='...'
```

Properties worth knowing:

- **Written before the listener is bound.** A healthy container therefore also means the file is
  on disk, so consumers can simply wait on `condition: service_healthy`. The image ships a
  `HEALTHCHECK` that polls `/health`.
- **Atomic.** The file is written to a temporary path and renamed, so a reader never sees a
  partial write.
- **Retried.** The guest agent is queried up to `RETRIES + 1` times, spaced by `RETRY_DELAY_MS`.
- **Fail-fast by default.** If the file cannot be written, startup aborts with a non-zero exit
  code. This is what keeps the guarantee above meaningful: a healthy container always has a
  fresh file.
- Values are single-quoted and escaped, so they are safe to `source` from a POSIX shell.
- `app_cert` and `tcb_info` are deliberately **not** exported. Use `GET /info` for the full payload.

> **Careful with `REQUIRED=false`.** It downgrades a write failure to a warning and lets the
> service start, which breaks the healthy-implies-written guarantee in two ways: consumers that
> `source` the file will fail on a missing file, and if a file from a previous boot is still on
> the volume they will silently label their records with a **stale** `instance_id`. Only use it
> when serving quotes matters more than labelling logs correctly.

### Fluent Bit integration

```yaml
services:
  dstack-quote-service:
    image: docker-regis.iex.ec/dstack-quote-service:<tag>
    environment:
      QUOTE_SIDECAR_INSTANCE_FILE__ENABLED: "true"
      QUOTE_SIDECAR_INSTANCE_FILE__PATH: /shared/instance.env
    volumes:
      - /var/run/dstack.sock:/var/run/dstack.sock
      - shared:/shared

  fluentbit:
    image: fluent/fluent-bit:latest
    depends_on:
      dstack-quote-service:
        condition: service_healthy
    entrypoint: ["sh", "-c"]
    command:
      - . /shared/instance.env
        && export INSTANCE_ID APP_ID APP_NAME COMPOSE_HASH
        && exec /fluent-bit/bin/fluent-bit -c /fluent-bit/etc/fluent-bit.conf
    volumes:
      - shared:/shared
      - ./fluent-bit.conf:/fluent-bit/etc/fluent-bit.conf

volumes:
  shared:
```

The values are then usable in `fluent-bit.conf`:

```ini
[FILTER]
    Name          record_modifier
    Match         *
    Record        instance_id ${INSTANCE_ID}
    Record        app_name    ${APP_NAME}
```

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

To exercise the instance file as well:

```bash
export QUOTE_SIDECAR_INSTANCE_FILE__ENABLED=true
export QUOTE_SIDECAR_INSTANCE_FILE__PATH=/tmp/dstack-test/instance.env
cargo run
```

`Instance file written to ...` is logged before `Server bound to ...`.

### 5. Test the Endpoints

```bash
# Test quote endpoint
curl "http://localhost:9999/quote?data=test123"

# Test attestation endpoint
curl "http://localhost:9999/attest?data=my-app-state"

# Test info endpoint
curl -s http://localhost:9999/info | jq -r .instance_id

# Check the instance file is sourceable
sh -c '. /tmp/dstack-test/instance.env && echo "$INSTANCE_ID / $APP_NAME"'
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

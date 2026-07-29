//! Startup persistence of the CVM identity as a Fluent Bit config fragment.
//!
//! Fluent Bit, running alongside this service inside the CVM, needs the dstack
//! `instance_id` to label the records it forwards. Rather than giving it its own
//! access to the dstack socket, this service — which already talks to the guest
//! agent — writes the identity once at startup to a file on a shared volume.
//!
//! The file holds `@SET` directives, which Fluent Bit reads through an
//! `@INCLUDE`. This matters because the official Fluent Bit images are
//! distroless: with no shell in them, sourcing an `.env` file from an entrypoint
//! override is not an option.
//!
//! The file is written before the HTTP listener is bound, so a successful
//! `/health` response also guarantees that the file is present. Fluent Bit can
//! therefore wait on `depends_on: condition: service_healthy`.

use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use dstack_sdk::dstack_client::{DstackClient, InfoResponse};
use tokio::fs;
use tokio::time::{Duration, sleep};
use tracing::warn;

use crate::config::InstanceFileConfig;

/// Queries the dstack guest agent and persists the CVM identity to `cfg.path`.
///
/// The file is written atomically, so a concurrent reader either sees the
/// previous content or the complete new one, never a partial write.
pub async fn write(client: &DstackClient, cfg: &InstanceFileConfig) -> Result<()> {
    let info = fetch_info(client, cfg).await?;
    let contents = render(&info)?;
    write_atomically(&cfg.path, &contents).await
}

/// Calls the guest agent `Info` endpoint, retrying on failure.
///
/// The socket may not be ready yet when the container starts, so the initial
/// attempt is followed by up to `cfg.retries` additional ones.
async fn fetch_info(client: &DstackClient, cfg: &InstanceFileConfig) -> Result<InfoResponse> {
    let attempts = cfg.retries.saturating_add(1);
    let mut attempt = 1;

    loop {
        let err = match client.info().await {
            Ok(info) => return Ok(info),
            Err(err) => err,
        };

        if attempt >= attempts {
            return Err(err).with_context(|| {
                format!("Failed to query the dstack guest agent after {attempts} attempt(s)")
            });
        }

        warn!(
            "Failed to query the dstack guest agent (attempt {attempt}/{attempts}), \
             retrying in {}ms: {err:#}",
            cfg.retry_delay_ms
        );
        sleep(Duration::from_millis(cfg.retry_delay_ms)).await;
        attempt += 1;
    }
}

/// The identity fields exported to the instance file, in a stable order.
///
/// Only the fields useful as log labels are here. `app_cert` (a multi-kilobyte
/// PEM blob) and `tcb_info` (a nested object) are deliberately left out; `GET
/// /info` serves the full payload for callers that need it.
fn exported_fields(info: &InfoResponse) -> [(&'static str, &str); 4] {
    [
        ("INSTANCE_ID", &info.instance_id),
        ("APP_ID", &info.app_id),
        ("APP_NAME", &info.app_name),
        ("COMPOSE_HASH", &info.compose_hash),
    ]
}

/// Renders the exported fields as Fluent Bit `@SET` directives.
///
/// Values are written bare: `@SET` takes everything up to the end of the line
/// literally, so quoting them would make the quotes part of the value.
///
/// # Errors
///
/// Returns an error if a value contains a newline. It would end the directive
/// early and turn the remainder into a stray configuration line, which Fluent
/// Bit either refuses to start on or, worse, interprets.
fn render(info: &InfoResponse) -> Result<String> {
    let mut rendered = String::new();

    for (key, value) in exported_fields(info) {
        if value.contains('\n') {
            bail!("Guest agent returned a {key} containing a newline, which cannot be exported");
        }
        rendered.push_str(&format!("@SET {key}={value}\n"));
    }

    Ok(rendered)
}

/// Writes `contents` to `path` through a temporary file and a rename.
///
/// The rename is atomic because both paths live in the same directory, hence on
/// the same filesystem. Missing parent directories are created.
async fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    fs::create_dir_all(dir)
        .await
        .with_context(|| format!("Failed to create directory {}", dir.display()))?;

    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("Instance file path {} has no file name", path.display()))?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(".tmp");
    let tmp_path = dir.join(tmp_name);

    fs::write(&tmp_path, contents)
        .await
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;

    if let Err(err) = fs::rename(&tmp_path, path).await {
        // Best effort: never leave a stale temporary file behind for a sidecar to trip on.
        let _ = fs::remove_file(&tmp_path).await;
        return Err(err).with_context(|| {
            format!(
                "Failed to move {} to {}",
                tmp_path.display(),
                path.display()
            )
        });
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Builds an `InfoResponse` fixture, going through deserialization so the
    /// fixture stays honest with respect to the SDK type.
    fn info_fixture(
        instance_id: &str,
        app_id: &str,
        app_name: &str,
        compose_hash: &str,
    ) -> InfoResponse {
        serde_json::from_value(json!({
            "app_id": app_id,
            "instance_id": instance_id,
            "app_cert": "-----BEGIN CERTIFICATE-----\nirrelevant\n-----END CERTIFICATE-----",
            "app_name": app_name,
            "device_id": "device-id",
            "key_provider_info": "{}",
            "compose_hash": compose_hash,
            "tcb_info": {
                "mrtd": "",
                "rtmr0": "",
                "rtmr1": "",
                "rtmr2": "",
                "rtmr3": "",
                "compose_hash": compose_hash,
                "device_id": "device-id",
                "app_compose": "{}",
                "event_log": []
            }
        }))
        .expect("the InfoResponse fixture must deserialize")
    }

    /// Returns a unique, non-existent directory under the system temp directory.
    fn unique_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "dstack-quote-service-{label}-{}-{seq}",
            std::process::id()
        ))
    }

    mod render {
        use super::*;

        #[test]
        fn should_emit_the_four_exported_keys_in_a_stable_order() {
            let info = info_fixture("i-1", "app-1", "my-app", "hash-1");

            let result = render(&info).expect("render must succeed");

            assert_eq!(
                result,
                "@SET INSTANCE_ID=i-1\n@SET APP_ID=app-1\n@SET APP_NAME=my-app\n@SET COMPOSE_HASH=hash-1\n"
            );
        }

        #[test]
        fn should_write_values_unquoted() {
            let info = info_fixture("i-1", "app-1", "it's an app", "hash-1");

            let result = render(&info).expect("render must succeed");

            assert!(
                result.contains("@SET APP_NAME=it's an app\n"),
                "@SET takes the value literally, quotes would become part of it: {result}"
            );
        }

        #[test]
        fn should_reject_a_value_containing_a_newline() {
            let info = info_fixture("i-1", "app-1", "evil\n@SET INSTANCE_ID=spoofed", "hash-1");

            let result = render(&info);

            assert!(result.is_err(), "a newline must not reach the file");
        }

        #[test]
        fn should_not_leak_the_app_certificate() {
            let info = info_fixture("i-1", "app-1", "my-app", "hash-1");

            let result = render(&info).expect("render must succeed");

            assert!(
                !result.contains("BEGIN CERTIFICATE"),
                "app_cert must never reach the instance file: {result}"
            );
        }
    }

    mod write_atomically {
        use super::*;

        #[tokio::test]
        async fn should_create_the_missing_parent_directory() {
            let dir = unique_temp_dir("create-parent");
            let path = dir.join("instance.conf");

            write_atomically(&path, "@SET INSTANCE_ID=i-1\n")
                .await
                .expect("write must succeed");
            let written = std::fs::read_to_string(&path).expect("file must exist");
            let _ = std::fs::remove_dir_all(&dir);

            assert_eq!(written, "@SET INSTANCE_ID=i-1\n");
        }

        #[tokio::test]
        async fn should_leave_no_temporary_file_behind() {
            let dir = unique_temp_dir("no-temp-leftover");
            let path = dir.join("instance.conf");

            write_atomically(&path, "@SET INSTANCE_ID=i-1\n")
                .await
                .expect("write must succeed");
            let leftovers: Vec<PathBuf> = std::fs::read_dir(&dir)
                .expect("directory must exist")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|entry| entry != &path)
                .collect();
            let _ = std::fs::remove_dir_all(&dir);

            assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
        }

        #[tokio::test]
        async fn should_overwrite_an_existing_file() {
            let dir = unique_temp_dir("overwrite");
            let path = dir.join("instance.conf");

            write_atomically(&path, "@SET INSTANCE_ID=old\n")
                .await
                .expect("first write must succeed");
            write_atomically(&path, "@SET INSTANCE_ID=new\n")
                .await
                .expect("second write must succeed");
            let written = std::fs::read_to_string(&path).expect("file must exist");
            let _ = std::fs::remove_dir_all(&dir);

            assert_eq!(written, "@SET INSTANCE_ID=new\n");
        }
    }
}

//! Uploading a set of files to a [`PublishTarget`].
//!
//! Ordering and atomicity: each file is written under a temporary name and renamed
//! into place when the backend can rename (local folders, SFTP). Backends without
//! rename (S3) get a direct write, which S3 applies atomically per object. Files
//! upload in order: everything else first, then `manifest.json`, then `index.html`
//! last, so a reader that finds the index also finds the data it links to. A
//! publish is not atomic as a whole; a failure part way leaves earlier files in place.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use opendal::Operator;

use crate::backend;
use crate::error::PublishError;
use crate::manifest::{MANIFEST_NAME, sha256_file, sha256_hex};
use crate::names::validate_names;
use crate::target::{Env, PublishTarget};

/// The entry page. Uploaded last.
pub const INDEX_NAME: &str = "index.html";

/// The bytes of one published file: in memory, or read from a local path at upload time.
#[derive(Debug, Clone)]
pub enum Content {
    Bytes(Vec<u8>),
    Path(PathBuf),
}

impl Content {
    /// Size in bytes and lowercase hex SHA-256.
    pub fn size_and_sha256(&self) -> Result<(u64, String), PublishError> {
        match self {
            Content::Bytes(bytes) => Ok((bytes.len() as u64, sha256_hex(bytes))),
            Content::Path(path) => sha256_file(path)
                .map_err(|e| PublishError::Io(format!("cannot read '{}': {e}", path.display()))),
        }
    }
}

/// One file to publish under a validated relative name.
#[derive(Debug, Clone)]
pub struct FileToPublish {
    pub name: String,
    pub content: Content,
}

/// One file that was uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub name: String,
    pub bytes: u64,
}

/// The result of a successful publish.
#[derive(Debug, Clone)]
pub struct PublishReport {
    /// The target, without credentials.
    pub target: String,
    /// Files in upload order.
    pub uploaded: Vec<UploadedFile>,
}

/// Checks names, count and duplicates for a list of files.
pub fn validate_files(files: &[FileToPublish]) -> Result<(), PublishError> {
    validate_names(files.iter().map(|f| f.name.as_str()))
}

/// Uploads `files` to `target`. Credentials come from `env`. Errors never contain
/// secret values from `env`.
pub fn publish(
    files: &[FileToPublish],
    target: &PublishTarget,
    env: &Env,
) -> Result<PublishReport, PublishError> {
    publish_with_options(files, target, env, &PublishOptions::default())
}

/// Options for [`publish_with_options`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PublishOptions {
    /// SFTP only: record an unknown server host key in known_hosts on first use.
    pub trust_new_host_key: bool,
}

/// Like [`publish`], with options.
pub fn publish_with_options(
    files: &[FileToPublish],
    target: &PublishTarget,
    env: &Env,
    options: &PublishOptions,
) -> Result<PublishReport, PublishError> {
    let secrets = env.secret_values();
    let label = target.describe();
    let outcome = validate_files(files).and_then(|()| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| PublishError::Io(format!("cannot start the async runtime: {e}")))?;
        match target {
            PublishTarget::Sftp {
                user,
                host,
                port,
                path,
            } => {
                let endpoint = crate::sftp::SftpEndpoint {
                    user: user.as_deref(),
                    host,
                    port: *port,
                    root: path,
                };
                let policy = crate::sftp::HostKeyPolicy {
                    trust_new_host_key: options.trust_new_host_key,
                };
                let uploaded =
                    runtime.block_on(crate::sftp::upload(&endpoint, files, env, policy))?;
                Ok(PublishReport {
                    target: label.clone(),
                    uploaded,
                })
            }
            _ => {
                let op = backend::operator(target, env)?;
                runtime.block_on(publish_with_operator(&op, &label, files))
            }
        }
    });
    outcome.map_err(|e| e.redacted(&secrets))
}

/// Uploads through an existing OpenDAL operator. Public so tests and embedders can
/// use the in-memory or fs services; [`publish`] is the normal entry point.
pub async fn publish_with_operator(
    op: &Operator,
    label: &str,
    files: &[FileToPublish],
) -> Result<PublishReport, PublishError> {
    validate_files(files)?;
    let can_rename = op.info().capability().rename;
    let mut uploaded = Vec::with_capacity(files.len());
    for file in upload_order(files) {
        let data = match &file.content {
            Content::Bytes(bytes) => bytes.clone(),
            Content::Path(path) => tokio::fs::read(path)
                .await
                .map_err(|e| PublishError::Io(format!("cannot read '{}': {e}", path.display())))?,
        };
        let bytes = data.len() as u64;
        if can_rename {
            write_then_rename(op, &file.name, data).await?;
        } else {
            op.write(&file.name, data)
                .await
                .map_err(|e| backend_error(&file.name, e))?;
        }
        uploaded.push(UploadedFile {
            name: file.name.clone(),
            bytes,
        });
    }
    Ok(PublishReport {
        target: label.to_string(),
        uploaded,
    })
}

/// Files in upload order: others, then the manifest, then the index.
pub(crate) fn upload_order(files: &[FileToPublish]) -> Vec<&FileToPublish> {
    let rank = |name: &str| match name {
        INDEX_NAME => 2,
        MANIFEST_NAME => 1,
        _ => 0,
    };
    let mut ordered: Vec<&FileToPublish> = files.iter().collect();
    ordered.sort_by_key(|f| rank(&f.name));
    ordered
}

async fn write_then_rename(op: &Operator, name: &str, data: Vec<u8>) -> Result<(), PublishError> {
    let temp = temp_name(name);
    op.write(&temp, data)
        .await
        .map_err(|e| backend_error(name, e))?;
    if op.rename(&temp, name).await.is_err() {
        // Some servers refuse to rename over an existing file. Remove the old
        // version and try once more; the window is small and only on those servers.
        if op.exists(name).await.unwrap_or(false) {
            let _ = op.delete(name).await;
        }
        if let Err(e) = op.rename(&temp, name).await {
            let _ = op.delete(&temp).await;
            return Err(backend_error(name, e));
        }
    }
    Ok(())
}

/// A hidden temporary name next to `name`, unique per process and call.
pub(crate) fn temp_name(name: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let (dir, file) = match name.rfind('/') {
        Some(index) => (&name[..=index], &name[index + 1..]),
        None => ("", name),
    };
    format!("{dir}.{file}.part-{}-{nanos:x}-{seq}", std::process::id())
}

fn backend_error(name: &str, err: opendal::Error) -> PublishError {
    PublishError::Backend(format!("'{name}': {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &str) -> FileToPublish {
        FileToPublish {
            name: name.into(),
            content: Content::Bytes(text.as_bytes().to_vec()),
        }
    }

    #[test]
    fn index_is_uploaded_last_and_manifest_before_it() {
        let files = [
            file("index.html", "i"),
            file("manifest.json", "m"),
            file("b.csv", "b"),
            file("a.csv", "a"),
        ];
        let order: Vec<&str> = upload_order(&files)
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(order, ["b.csv", "a.csv", "manifest.json", "index.html"]);
    }

    #[test]
    fn temp_names_stay_in_the_same_folder_and_are_hidden() {
        let temp = temp_name("charts/revenue.png");
        assert!(temp.starts_with("charts/.revenue.png.part-"), "{temp}");
        let other = temp_name("revenue.png");
        assert!(other.starts_with(".revenue.png.part-"), "{other}");
        assert_ne!(temp, temp_name("charts/revenue.png"));
    }

    #[test]
    fn traversal_is_refused_before_anything_is_written() {
        let files = [file("index.html", "x"), file("../escape.html", "y")];
        let err = validate_files(&files).unwrap_err();
        assert!(matches!(err, PublishError::InvalidName { .. }), "{err}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn memory_backend_gets_direct_writes_and_all_files() {
        let op = Operator::new(opendal::services::Memory::default()).unwrap();
        let files = [file("index.html", "<html>"), file("data.csv", "a,b\n1,2\n")];
        let report = publish_with_operator(&op, "memory", &files).await.unwrap();
        assert_eq!(report.uploaded.len(), 2);
        assert_eq!(report.uploaded.last().unwrap().name, "index.html");
        assert_eq!(op.read("data.csv").await.unwrap().to_vec(), b"a,b\n1,2\n");
        assert_eq!(op.read("index.html").await.unwrap().to_vec(), b"<html>");
    }
}

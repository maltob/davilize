//! Publishing and serving for Davilize.
//!
//! * [`PublishTarget`] parses a destination: a local folder (`file://` or a plain
//!   path), `s3://bucket/prefix` (also S3-compatible stores through
//!   `AWS_ENDPOINT_URL`), or `sftp://user@host[:port]/path` (pure Rust, always built in).
//! * [`publish`] uploads a set of files through Apache OpenDAL. The async work runs on
//!   a small tokio runtime inside the call, so callers stay synchronous.
//! * [`manifest`] builds `manifest.json` (file list, sizes, SHA-256, report name and
//!   version, generation time).
//! * [`serve`] is a static server for a published folder, with an optional refresh
//!   endpoint and atomic root swapping.
//!
//! Credentials come only from the environment, never from URLs. See docs/publishing.md.

pub mod error;
pub mod manifest;
pub mod names;
pub mod publish;
pub mod serve;
pub mod sftp;
pub mod target;

mod backend;

pub use error::PublishError;
pub use manifest::{MANIFEST_NAME, build_manifest, sha256_hex};
pub use names::{MAX_FILES, validate_relative_name};
pub use publish::{
    Content, FileToPublish, INDEX_NAME, PublishOptions, PublishReport, UploadedFile, publish,
    publish_with_operator, publish_with_options, validate_files,
};
pub use target::{Env, PublishTarget};

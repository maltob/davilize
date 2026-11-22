//! Builds OpenDAL operators for local folders and S3. Credentials are read here from the
//! [`Env`]; nothing is read from URLs. Building an operator does not connect. SFTP does
//! not use OpenDAL (see `sftp.rs`).

use opendal::{Operator, services};

use crate::error::PublishError;
use crate::target::{Env, PublishTarget};

pub(crate) fn operator(target: &PublishTarget, env: &Env) -> Result<Operator, PublishError> {
    match target {
        PublishTarget::Local { root } => local_operator(root),
        PublishTarget::S3 { bucket, prefix } => s3_operator(bucket, prefix, env),
        PublishTarget::Sftp { .. } => Err(PublishError::Config(
            "internal: SFTP targets are uploaded by sftp::upload, not through OpenDAL".into(),
        )),
    }
}

fn local_operator(root: &std::path::Path) -> Result<Operator, PublishError> {
    std::fs::create_dir_all(root)
        .map_err(|e| PublishError::Io(format!("cannot create '{}': {e}", root.display())))?;
    let builder = services::Fs::default().root(&root.to_string_lossy());
    Operator::new(builder).map_err(|e| PublishError::Config(format!("local target: {e}")))
}

/// S3 and S3-compatible stores (MinIO, Ceph, R2, ...).
///
/// Environment: `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (both or neither;
/// neither means the default AWS chain: profile, SSO, instance role),
/// `AWS_SESSION_TOKEN` (optional), `AWS_REGION` or `AWS_DEFAULT_REGION` (default
/// `us-east-1`), and `AWS_ENDPOINT_URL_S3` or `AWS_ENDPOINT_URL` for a custom endpoint.
fn s3_operator(bucket: &str, prefix: &str, env: &Env) -> Result<Operator, PublishError> {
    let endpoint = env
        .get("AWS_ENDPOINT_URL_S3")
        .or_else(|| env.get("AWS_ENDPOINT_URL"));
    let region = env
        .get("AWS_REGION")
        .or_else(|| env.get("AWS_DEFAULT_REGION"))
        .unwrap_or("us-east-1");
    let root = if prefix.is_empty() {
        "/".to_string()
    } else {
        format!("/{prefix}/")
    };

    let mut builder = services::S3::default()
        .bucket(bucket)
        .region(region)
        .root(&root);
    builder = match endpoint {
        // Custom endpoints (MinIO and friends) use path-style addressing.
        Some(url) => builder.endpoint(url),
        // AWS itself: bucket as a subdomain.
        None => builder.enable_virtual_host_style(),
    };
    match (
        env.get("AWS_ACCESS_KEY_ID"),
        env.get("AWS_SECRET_ACCESS_KEY"),
    ) {
        (Some(id), Some(secret)) => {
            builder = builder.access_key_id(id).secret_access_key(secret);
            if let Some(token) = env.get("AWS_SESSION_TOKEN") {
                builder = builder.session_token(token);
            }
        }
        (None, None) => {}
        _ => {
            return Err(PublishError::Config(
                "AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY must be set together".into(),
            ));
        }
    }
    Operator::new(builder).map_err(|e| PublishError::Config(format!("S3 target: {e}")))
}

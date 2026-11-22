//! SFTP publishing with a pure-Rust SSH client (russh) and SFTP layer (russh-sftp).
//! No external ssh binary is used, so it works the same on Windows, macOS and Linux.
//!
//! Security rules:
//! * The server host key must match a record in known_hosts (default `~/.ssh/known_hosts`,
//!   or `DAVILIZE_KNOWN_HOSTS`). A key that does not match is always refused. An unknown
//!   host is refused unless the user passes `--trust-new-host-key`, which records the key
//!   on first use.
//! * Key auth reads `DAVILIZE_SFTP_KEY` (a private key file, with optional passphrase in
//!   `DAVILIZE_SFTP_KEY_PASSPHRASE`). Password auth uses `DAVILIZE_SFTP_PASSWORD` only.
//!   Nothing secret comes from the URL or the command line.
//! * Each file goes to a hidden temporary name and is renamed into place. The index
//!   uploads last (see `publish::upload_order`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::Disconnect;
use russh::client::AuthResult;
use russh::client::{self, Config, Handle};
use russh::keys::known_hosts::learn_known_hosts_path;
use russh::keys::{
    HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
    check_known_hosts_path, load_secret_key,
};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;

use crate::error::PublishError;
use crate::publish::{Content, FileToPublish, UploadedFile, temp_name, upload_order};
use crate::target::Env;

/// The parts of an `sftp://` target that the client needs.
pub(crate) struct SftpEndpoint<'a> {
    pub user: Option<&'a str>,
    pub host: &'a str,
    pub port: u16,
    pub root: &'a str,
}

/// Whether an unknown host key may be recorded on first use.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostKeyPolicy {
    pub trust_new_host_key: bool,
}

/// Decides whether `key` is acceptable for `host:port`, using the known_hosts file at
/// `known_hosts`. When `policy.trust_new_host_key` is set and the host is unknown, the key
/// is recorded. Mismatches are always refused. Returns the reason on refusal.
pub fn judge_host_key(
    host: &str,
    port: u16,
    key: &PublicKey,
    known_hosts: &Path,
    policy: HostKeyPolicy,
) -> Result<(), String> {
    let fingerprint = key.fingerprint(HashAlg::Sha256);
    match check_known_hosts_path(host, port, key, known_hosts) {
        Ok(true) => Ok(()),
        Ok(false) if policy.trust_new_host_key => {
            learn_known_hosts_path(host, port, key, known_hosts).map_err(|e| {
                format!(
                    "cannot record the host key for {host}:{port} in '{}': {e}",
                    known_hosts.display()
                )
            })?;
            Ok(())
        }
        Ok(false) => Err(format!(
            "host key for {host}:{port} is not in '{}' (fingerprint {fingerprint}). Verify the key out of band, add it to known_hosts, or pass --trust-new-host-key to record it on first use",
            known_hosts.display()
        )),
        Err(e) => Err(format!(
            "host key for {host}:{port} does not match '{}' (fingerprint {fingerprint}): {e}. The server may have been reinstalled, or someone may be intercepting the connection",
            known_hosts.display()
        )),
    }
}

/// Default known_hosts location, or `DAVILIZE_KNOWN_HOSTS` when set.
pub(crate) fn known_hosts_path(env: &Env) -> Result<PathBuf, PublishError> {
    if let Some(path) = env.get("DAVILIZE_KNOWN_HOSTS") {
        return Ok(PathBuf::from(path));
    }
    std::env::home_dir()
        .map(|home| home.join(".ssh").join("known_hosts"))
        .ok_or_else(|| {
            PublishError::Config(
                "cannot find the home folder for known_hosts; set DAVILIZE_KNOWN_HOSTS".into(),
            )
        })
}

struct Verifier {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    policy: HostKeyPolicy,
    /// Set when the key is refused, so the connection error can say why.
    refusal: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => {
                self.refuse("the server offered a host certificate, which is not supported");
                return Ok(false);
            }
        };
        match judge_host_key(&self.host, self.port, key, &self.known_hosts, self.policy) {
            Ok(()) => Ok(true),
            Err(reason) => {
                self.refuse(&reason);
                Ok(false)
            }
        }
    }
}

impl Verifier {
    fn refuse(&self, reason: &str) {
        if let Ok(mut slot) = self.refusal.lock() {
            *slot = Some(reason.to_string());
        }
    }
}

/// Uploads `files` to the SFTP endpoint. Returns the uploaded files in upload order.
pub(crate) async fn upload(
    endpoint: &SftpEndpoint<'_>,
    files: &[FileToPublish],
    env: &Env,
    policy: HostKeyPolicy,
) -> Result<Vec<UploadedFile>, PublishError> {
    let user = endpoint
        .user
        .or_else(|| env.get("DAVILIZE_SFTP_USER"))
        .ok_or_else(|| {
            PublishError::Config(
                "sftp needs a user name: write sftp://user@host/path or set DAVILIZE_SFTP_USER"
                    .into(),
            )
        })?;
    let known_hosts = known_hosts_path(env)?;
    let mut session = connect(endpoint, known_hosts, policy).await?;
    authenticate(&mut session, user, env).await?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|e| backend(format!("cannot open an SSH session: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| backend(format!("the server refused the sftp subsystem: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| backend(format!("SFTP handshake failed: {e}")))?;

    let root = endpoint.root.trim_end_matches('/');
    ensure_dirs(&sftp, root).await?;
    let mut uploaded = Vec::with_capacity(files.len());
    for file in upload_order(files) {
        let data = match &file.content {
            Content::Bytes(bytes) => bytes.clone(),
            Content::Path(path) => tokio::fs::read(path)
                .await
                .map_err(|e| PublishError::Io(format!("cannot read '{}': {e}", path.display())))?,
        };
        let dest = format!("{root}/{}", file.name);
        if let Some(index) = file.name.rfind('/') {
            ensure_dirs(&sftp, &format!("{root}/{}", &file.name[..index])).await?;
        }
        let temp = format!("{}/{}", root, temp_name(&file.name));
        write_file(&sftp, &temp, &data, &file.name).await?;
        rename_into_place(&sftp, &temp, &dest, &file.name).await?;
        uploaded.push(UploadedFile {
            name: file.name.clone(),
            bytes: data.len() as u64,
        });
    }
    let _ = sftp.close().await;
    let _ = session
        .disconnect(Disconnect::ByApplication, "publish finished", "en")
        .await;
    Ok(uploaded)
}

async fn connect(
    endpoint: &SftpEndpoint<'_>,
    known_hosts: PathBuf,
    policy: HostKeyPolicy,
) -> Result<Handle<Verifier>, PublishError> {
    let refusal = Arc::new(Mutex::new(None));
    let host = bare_host(endpoint.host);
    let handler = Verifier {
        host: host.to_string(),
        port: endpoint.port,
        known_hosts,
        policy,
        refusal: Arc::clone(&refusal),
    };
    let config = Arc::new(Config {
        inactivity_timeout: Some(Duration::from_secs(60)),
        ..Config::default()
    });
    match client::connect(config, (host, endpoint.port), handler).await {
        Ok(session) => Ok(session),
        Err(err) => {
            let reason = refusal.lock().ok().and_then(|mut slot| slot.take());
            Err(backend(reason.unwrap_or_else(|| {
                format!("cannot connect to {host}:{}: {err}", endpoint.port)
            })))
        }
    }
}

/// The URL parser keeps the brackets of an IPv6 literal (`[::1]`); the socket address
/// and known_hosts lookup need the bare address.
fn bare_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

async fn authenticate(
    session: &mut Handle<Verifier>,
    user: &str,
    env: &Env,
) -> Result<(), PublishError> {
    if let Some(path) = env.get("DAVILIZE_SFTP_KEY") {
        let key = load_key(path, env.get("DAVILIZE_SFTP_KEY_PASSPHRASE"))?;
        if try_key(session, user, key).await? {
            return Ok(());
        }
    }
    if let Some(password) = env.get("DAVILIZE_SFTP_PASSWORD") {
        let result = session
            .authenticate_password(user, password)
            .await
            .map_err(|e| backend(format!("password authentication failed: {e}")))?;
        if matches!(result, AuthResult::Success) {
            return Ok(());
        }
    }
    Err(backend(format!(
        "the server did not accept the credentials for user '{user}'. Set DAVILIZE_SFTP_KEY (and DAVILIZE_SFTP_KEY_PASSPHRASE if the key is encrypted), or DAVILIZE_SFTP_PASSWORD"
    )))
}

fn load_key(path: &str, passphrase: Option<&str>) -> Result<PrivateKey, PublishError> {
    load_secret_key(path, passphrase)
        .map_err(|e| PublishError::Config(format!("cannot load the SFTP key '{path}': {e}")))
}

async fn try_key(
    session: &mut Handle<Verifier>,
    user: &str,
    key: PrivateKey,
) -> Result<bool, PublishError> {
    // Modern servers reject RSA keys with SHA-1 signatures, so request SHA-512 for RSA.
    let hash = key.algorithm().is_rsa().then_some(HashAlg::Sha512);
    let with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    let result = session
        .authenticate_publickey(user, with_hash)
        .await
        .map_err(|e| backend(format!("public key authentication failed: {e}")))?;
    Ok(matches!(result, AuthResult::Success))
}

/// Creates each missing directory of `path` (absolute, no trailing slash).
async fn ensure_dirs(sftp: &SftpSession, path: &str) -> Result<(), PublishError> {
    let mut current = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current.push('/');
        current.push_str(part);
        if sftp.metadata(current.as_str()).await.is_err() {
            sftp.create_dir(current.as_str())
                .await
                .map_err(|e| backend(format!("cannot create remote folder '{current}': {e}")))?;
        }
    }
    Ok(())
}

async fn write_file(
    sftp: &SftpSession,
    temp: &str,
    data: &[u8],
    name: &str,
) -> Result<(), PublishError> {
    let mut file = sftp
        .create(temp)
        .await
        .map_err(|e| backend(format!("'{name}': cannot create a temporary file: {e}")))?;
    file.write_all(data)
        .await
        .map_err(|e| backend(format!("'{name}': write failed: {e}")))?;
    file.shutdown()
        .await
        .map_err(|e| backend(format!("'{name}': cannot close the file: {e}")))?;
    Ok(())
}

async fn rename_into_place(
    sftp: &SftpSession,
    temp: &str,
    dest: &str,
    name: &str,
) -> Result<(), PublishError> {
    if sftp.rename(temp, dest).await.is_ok() {
        return Ok(());
    }
    // Some servers refuse to rename over an existing file: remove the old one and retry.
    let _ = sftp.remove_file(dest).await;
    if let Err(e) = sftp.rename(temp, dest).await {
        let _ = sftp.remove_file(temp).await;
        return Err(backend(format!("'{name}': rename into place failed: {e}")));
    }
    Ok(())
}

fn backend(message: String) -> PublishError {
    PublishError::Backend(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two ed25519 public keys generated for these tests (no private key is kept in the repo).
    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEXJNlvIpiymwjaKg/5HQQX9zDFGO0BzBgbgK54RnXcN";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIANbJaHLk7NKbX7ReKI1yV+f/+PdBmKdZ71IUhW7ymfo";

    fn key(line: &str) -> PublicKey {
        PublicKey::from_openssh(line).expect("valid test key")
    }

    #[test]
    fn unknown_host_is_refused_by_default_and_nothing_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let err = judge_host_key(
            "files.example.org",
            22,
            &key(KEY_A),
            &file,
            HostKeyPolicy::default(),
        )
        .unwrap_err();
        assert!(err.contains("not in"), "{err}");
        assert!(err.contains("--trust-new-host-key"), "{err}");
        assert!(!file.exists() || std::fs::read_to_string(&file).unwrap().is_empty());
    }

    #[test]
    fn trust_on_first_use_records_the_key_then_accepts_it_and_refuses_a_changed_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("known_hosts");
        let tofu = HostKeyPolicy {
            trust_new_host_key: true,
        };
        judge_host_key("files.example.org", 2222, &key(KEY_A), &file, tofu).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("files.example.org")
        );

        // The recorded key now matches, even without the trust flag.
        judge_host_key(
            "files.example.org",
            2222,
            &key(KEY_A),
            &file,
            HostKeyPolicy::default(),
        )
        .unwrap();

        // A different key for the same host is refused, even with the trust flag.
        let err = judge_host_key("files.example.org", 2222, &key(KEY_B), &file, tofu).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn ipv6_literals_lose_their_brackets() {
        assert_eq!(bare_host("[::1]"), "::1");
        assert_eq!(bare_host("example.com"), "example.com");
    }

    #[test]
    fn known_hosts_override_comes_from_the_environment() {
        let env = Env::from_pairs([("DAVILIZE_KNOWN_HOSTS", "/tmp/custom_known_hosts")]);
        assert_eq!(
            known_hosts_path(&env).unwrap(),
            PathBuf::from("/tmp/custom_known_hosts")
        );
    }
}

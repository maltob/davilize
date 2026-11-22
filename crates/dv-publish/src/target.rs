//! Publish targets parsed from a URL or a plain local path, and the environment
//! that supplies credentials. URLs never carry secrets: a password in the URL is
//! rejected, and keys come from the environment (see docs/publishing.md).

use std::collections::BTreeMap;
use std::path::PathBuf;

use percent_encoding::percent_decode_str;
use url::Url;

use crate::error::PublishError;

/// Environment variables this crate reads. Only these are copied from the process.
const ENV_KEYS: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_ENDPOINT_URL",
    "AWS_ENDPOINT_URL_S3",
    "DAVILIZE_SFTP_KEY",
    "DAVILIZE_SFTP_KEY_PASSPHRASE",
    "DAVILIZE_SFTP_PASSWORD",
    "DAVILIZE_SFTP_USER",
    "DAVILIZE_KNOWN_HOSTS",
    "DAVILIZE_REFRESH_TOKEN",
];

/// Variables whose values must never appear in logs or errors.
const SECRET_KEYS: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "DAVILIZE_SFTP_KEY_PASSPHRASE",
    "DAVILIZE_SFTP_PASSWORD",
    "DAVILIZE_REFRESH_TOKEN",
];

/// A destination for published files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishTarget {
    /// A folder on this machine (`file:///path`, a plain path, or a Windows path).
    Local { root: PathBuf },
    /// An S3 bucket, optionally under a key prefix. Region and endpoint (for
    /// S3-compatible stores such as MinIO) come from the environment.
    S3 { bucket: String, prefix: String },
    /// An SFTP server. The port defaults to 22. Host keys are always verified
    /// against known_hosts.
    Sftp {
        user: Option<String>,
        host: String,
        port: u16,
        path: String,
    },
}

impl PublishTarget {
    /// Parses a target. A string without `://` is a local path.
    pub fn parse(input: &str) -> Result<Self, PublishError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(invalid("the target is empty"));
        }
        if !input.contains("://") {
            return Ok(PublishTarget::Local {
                root: PathBuf::from(input),
            });
        }
        // The url crate resolves dot-segments silently, so check the raw text first:
        // a '..' in a target is almost always a mistake, and should be reported.
        let decoded_once = percent_decode_str(input).decode_utf8_lossy();
        if decoded_once.split('/').any(|s| s == "." || s == "..") {
            return Err(invalid("the path must not contain '.' or '..' segments"));
        }
        // Parse errors from the url crate do not echo the input, so they are safe to show.
        let url = Url::parse(input).map_err(|_| invalid("not a valid URL"))?;
        if url.password().is_some() {
            return Err(invalid(
                "passwords must not appear in the URL; use the environment (see docs/publishing.md)",
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("query strings and fragments are not supported"));
        }
        match url.scheme() {
            "file" => {
                let root = url
                    .to_file_path()
                    .map_err(|()| invalid("file:// URLs must name an absolute local path"))?;
                Ok(PublishTarget::Local { root })
            }
            "s3" => {
                let bucket = url
                    .host_str()
                    .filter(|b| !b.is_empty())
                    .ok_or_else(|| invalid("an s3:// URL needs a bucket name"))?
                    .to_string();
                validate_bucket(&bucket)?;
                let prefix = decode_path(url.path())?;
                Ok(PublishTarget::S3 { bucket, prefix })
            }
            "sftp" => {
                let host = url
                    .host_str()
                    .filter(|h| !h.is_empty())
                    .ok_or_else(|| invalid("an sftp:// URL needs a host"))?
                    .to_string();
                let user = match url.username() {
                    "" => None,
                    name => Some(decode_text(name)?),
                };
                let port = url.port().unwrap_or(22);
                let path = format!("/{}", decode_path(url.path())?);
                Ok(PublishTarget::Sftp {
                    user,
                    host,
                    port,
                    path,
                })
            }
            other => Err(invalid(&format!(
                "unsupported scheme '{other}:' (use a local path, file://, s3:// or sftp://)"
            ))),
        }
    }

    /// A human-readable form with no credentials, for logs and dry runs.
    pub fn describe(&self) -> String {
        match self {
            PublishTarget::Local { root } => root.display().to_string(),
            PublishTarget::S3 { bucket, prefix } => {
                if prefix.is_empty() {
                    format!("s3://{bucket}")
                } else {
                    format!("s3://{bucket}/{prefix}")
                }
            }
            PublishTarget::Sftp {
                user,
                host,
                port,
                path,
            } => {
                let user = user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
                format!("sftp://{user}{host}:{port}{path}")
            }
        }
    }
}

fn invalid(message: &str) -> PublishError {
    PublishError::InvalidTarget(message.to_string())
}

fn decode_text(raw: &str) -> Result<String, PublishError> {
    percent_decode_str(raw)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| invalid("the URL is not valid UTF-8 after percent-decoding"))
}

/// Decodes a URL path into a path without its outer slashes, rejecting traversal
/// segments, backslashes and control characters.
fn decode_path(raw: &str) -> Result<String, PublishError> {
    let decoded = decode_text(raw)?;
    let trimmed = decoded.trim_matches('/').to_string();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    for segment in trimmed.split('/') {
        if segment.is_empty() {
            return Err(invalid("the path contains an empty segment"));
        }
        if segment == "." || segment == ".." {
            return Err(invalid("the path must not contain '.' or '..' segments"));
        }
        if segment.contains('\\') || segment.chars().any(char::is_control) {
            return Err(invalid(
                "the path contains a backslash or control character",
            ));
        }
    }
    Ok(trimmed)
}

/// S3 bucket naming: 3 to 63 characters of lowercase letters, digits, dots and hyphens.
fn validate_bucket(bucket: &str) -> Result<(), PublishError> {
    let ok_chars = bucket
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    let len_ok = (3..=63).contains(&bucket.len());
    let edges_ok = !bucket.starts_with(['-', '.']) && !bucket.ends_with(['-', '.']);
    if ok_chars && len_ok && edges_ok {
        Ok(())
    } else {
        Err(invalid("the bucket name is not a valid S3 bucket name"))
    }
}

/// Settings and credentials read from the environment. Built once per publish;
/// tests construct one from pairs instead of touching the process environment.
#[derive(Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    /// Copies the variables this crate uses from the process environment.
    pub fn from_process() -> Self {
        let vars = ENV_KEYS
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v)))
            .collect();
        Env { vars }
    }

    /// Builds an environment from explicit pairs (tests and embedding).
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Env {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// The value of `key`, treating an empty value as unset.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// Secret values to scrub from any message shown to the user.
    pub fn secret_values(&self) -> Vec<String> {
        SECRET_KEYS
            .iter()
            .filter_map(|key| self.get(key).map(str::to_string))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> PublishTarget {
        PublishTarget::parse(input).unwrap_or_else(|e| panic!("{input}: {e}"))
    }

    fn parse_err(input: &str) -> String {
        match PublishTarget::parse(input) {
            Ok(t) => panic!("{input} should be rejected, parsed as {t:?}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn plain_paths_are_local() {
        assert_eq!(
            parse("out/site"),
            PublishTarget::Local {
                root: PathBuf::from("out/site")
            }
        );
        assert_eq!(
            parse("C:\\Users\\me\\site"),
            PublishTarget::Local {
                root: PathBuf::from("C:\\Users\\me\\site")
            }
        );
    }

    #[test]
    fn s3_urls_split_bucket_and_prefix() {
        assert_eq!(
            parse("s3://reports-bucket/team/q3/"),
            PublishTarget::S3 {
                bucket: "reports-bucket".into(),
                prefix: "team/q3".into()
            }
        );
        assert_eq!(
            parse("s3://reports-bucket"),
            PublishTarget::S3 {
                bucket: "reports-bucket".into(),
                prefix: String::new()
            }
        );
    }

    #[test]
    fn sftp_urls_carry_user_host_port_and_path() {
        assert_eq!(
            parse("sftp://deploy@example.com:2222/var/www/reports"),
            PublishTarget::Sftp {
                user: Some("deploy".into()),
                host: "example.com".into(),
                port: 2222,
                path: "/var/www/reports".into(),
            }
        );
        assert_eq!(
            parse("sftp://example.com/srv"),
            PublishTarget::Sftp {
                user: None,
                host: "example.com".into(),
                port: 22,
                path: "/srv".into(),
            }
        );
    }

    #[test]
    fn passwords_in_urls_are_rejected_without_echoing_them() {
        let msg = parse_err("sftp://deploy:hunter2@example.com/srv");
        assert!(msg.contains("passwords must not appear"), "{msg}");
        assert!(!msg.contains("hunter2"), "{msg}");
    }

    #[test]
    fn traversal_in_url_paths_is_rejected() {
        assert!(parse_err("sftp://h/a/../../etc").contains("'.' or '..'"));
        assert!(parse_err("s3://bucket-ok/%2e%2e/x").contains("'.' or '..'"));
        assert!(parse_err("sftp://h/a%5Cb").contains("backslash"));
    }

    #[test]
    fn query_strings_and_unknown_schemes_are_rejected() {
        assert!(parse_err("s3://bucket-ok/x?token=1").contains("query"));
        assert!(parse_err("https://example.com/x").contains("unsupported scheme"));
        assert!(parse_err("s3://Bad_Bucket").contains("bucket name"));
        assert!(parse_err("   ").contains("empty"));
    }

    #[test]
    fn file_urls_are_local() {
        #[cfg(windows)]
        let url = "file:///C:/Users/me/site";
        #[cfg(not(windows))]
        let url = "file:///tmp/site";
        assert!(matches!(parse(url), PublishTarget::Local { .. }));
    }

    #[test]
    fn describe_never_shows_a_password() {
        let target = parse("sftp://deploy@example.com:2222/srv");
        assert_eq!(target.describe(), "sftp://deploy@example.com:2222/srv");
    }

    #[test]
    fn env_treats_empty_as_unset_and_lists_secrets() {
        let env = Env::from_pairs([("AWS_SECRET_ACCESS_KEY", "abcdefgh"), ("AWS_REGION", "")]);
        assert_eq!(env.get("AWS_REGION"), None);
        assert_eq!(env.secret_values(), vec!["abcdefgh".to_string()]);
    }
}

//! Errors for publishing. Messages never contain secret values: every error
//! leaving [`crate::publish`] is passed through [`PublishError::redacted`].

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The target string could not be parsed (bad scheme, embedded password, ...).
    #[error("invalid publish target: {0}")]
    InvalidTarget(String),
    /// A relative file name was rejected (traversal, absolute, reserved, too long, ...).
    #[error("invalid file name '{name}': {reason}")]
    InvalidName { name: String, reason: String },
    /// Required configuration (environment variables) is missing or inconsistent.
    #[error("missing or inconsistent configuration: {0}")]
    Config(String),
    /// Local filesystem problem while reading the files to publish or preparing a folder.
    #[error("{0}")]
    Io(String),
    /// The storage backend failed: network, authentication, permissions, quota.
    #[error("publish failed: {0}")]
    Backend(String),
}

impl PublishError {
    /// Replaces every occurrence of a secret value in the message with `[redacted]`.
    pub(crate) fn redacted(self, secrets: &[String]) -> Self {
        let scrub = |text: String| redact_text(&text, secrets);
        match self {
            PublishError::InvalidTarget(s) => PublishError::InvalidTarget(scrub(s)),
            PublishError::InvalidName { name, reason } => PublishError::InvalidName {
                name: scrub(name),
                reason: scrub(reason),
            },
            PublishError::Config(s) => PublishError::Config(scrub(s)),
            PublishError::Io(s) => PublishError::Io(scrub(s)),
            PublishError::Backend(s) => PublishError::Backend(scrub(s)),
        }
    }
}

/// Replaces each non-empty secret in `text`. Very short values are skipped
/// because they would also mangle ordinary words.
pub(crate) fn redact_text(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 4 {
            out = out.replace(secret.as_str(), "[redacted]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_replaced_in_every_variant() {
        let secrets = vec!["s3cr3t-value".to_string(), "ab".to_string()];
        let err =
            PublishError::Backend("auth failed for key s3cr3t-value ab".into()).redacted(&secrets);
        let text = err.to_string();
        assert!(!text.contains("s3cr3t-value"), "{text}");
        // Short values are not used as patterns, so ordinary text survives.
        assert!(text.contains("ab"), "{text}");
    }
}

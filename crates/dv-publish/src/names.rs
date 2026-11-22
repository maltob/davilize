//! Validation of the relative names of published files. Names are written by
//! Davilize itself, but they are treated as untrusted: a name must stay inside the
//! target folder, so `..`, absolute paths, drive letters and backslashes are refused.

use std::collections::HashSet;

use crate::error::PublishError;

/// Maximum number of files in one publish.
pub const MAX_FILES: usize = 255;
/// Maximum length of a relative name, in bytes.
pub const MAX_NAME_LEN: usize = 512;
/// Maximum length of one path component, in bytes.
pub const MAX_COMPONENT_LEN: usize = 255;

/// Checks one relative name such as `revenue.csv` or `data/part-1.parquet`.
pub fn validate_relative_name(name: &str) -> Result<(), PublishError> {
    let fail = |reason: &str| PublishError::InvalidName {
        name: name.to_string(),
        reason: reason.to_string(),
    };
    if name.is_empty() {
        return Err(fail("the name is empty"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(fail("the name is too long"));
    }
    if name.starts_with('/') {
        return Err(fail("absolute paths are not allowed"));
    }
    if name.ends_with('/') {
        return Err(fail("the name must name a file, not a folder"));
    }
    if name.contains('\\') || name.contains(':') {
        return Err(fail("backslashes and colons are not allowed"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(fail(
            "only ASCII letters, digits, '-', '_', '.' and '/' are allowed",
        ));
    }
    for component in name.split('/') {
        if component == ".." {
            return Err(fail("path traversal ('..') is not allowed"));
        }
        if component.is_empty() || component == "." {
            return Err(fail("empty or '.' path components are not allowed"));
        }
        if component.starts_with('.') {
            return Err(fail("hidden (leading '.') components are not allowed"));
        }
        if component.len() > MAX_COMPONENT_LEN {
            return Err(fail("a path component is too long"));
        }
        if is_windows_reserved(component) {
            return Err(fail("the name is reserved on Windows"));
        }
    }
    Ok(())
}

/// Names such as `CON`, `NUL` or `COM1` (with or without an extension) cannot be
/// created on Windows, so they are refused everywhere to keep publishes portable.
fn is_windows_reserved(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Checks a list of names: each valid, no duplicates, and at most [`MAX_FILES`].
pub fn validate_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<(), PublishError> {
    let mut seen = HashSet::new();
    let mut count = 0usize;
    for name in names {
        count += 1;
        validate_relative_name(name)?;
        if !seen.insert(name) {
            return Err(PublishError::InvalidName {
                name: name.to_string(),
                reason: "the same name appears twice".into(),
            });
        }
    }
    if count > MAX_FILES {
        return Err(PublishError::InvalidName {
            name: format!("{count} files"),
            reason: format!("a publish may contain at most {MAX_FILES} files"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(name: &str) -> bool {
        validate_relative_name(name).is_err()
    }

    #[test]
    fn accepts_plain_and_nested_names() {
        for name in [
            "index.html",
            "revenue_by_region.csv",
            "data/part-1.parquet",
            "a/b/c.png",
        ] {
            assert!(validate_relative_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_path_traversal_in_every_spelling() {
        for name in [
            "../evil.html",
            "a/../../evil.html",
            "a/..",
            "..",
            "./x.csv",
            "a//b.csv",
            "a/./b.csv",
        ] {
            assert!(rejected(name), "{name} should be rejected");
        }
    }

    #[test]
    fn rejects_absolute_drive_and_backslash_names() {
        for name in [
            "/etc/passwd",
            "C:/Windows/x",
            "C:\\x",
            "..\\evil.html",
            "a\\b.csv",
            "dir/",
        ] {
            assert!(rejected(name), "{name} should be rejected");
        }
    }

    #[test]
    fn rejects_hidden_control_and_odd_characters() {
        for name in [
            ".hidden",
            "a/.env",
            "tab\there.csv",
            "nul\0.csv",
            "sp ace.csv",
            "é.csv",
            "",
        ] {
            assert!(rejected(name), "{name:?} should be rejected");
        }
    }

    #[test]
    fn rejects_windows_reserved_names_and_long_names() {
        assert!(rejected("CON.csv"));
        assert!(rejected("lpt3"));
        assert!(!rejected("COM10.csv"), "COM10 is not reserved");
        assert!(rejected(&format!("{}.csv", "x".repeat(MAX_COMPONENT_LEN))));
        assert!(rejected(&"a/".repeat(MAX_NAME_LEN)));
    }

    #[test]
    fn list_checks_duplicates_and_count() {
        assert!(validate_names(["a.csv", "a.csv"]).is_err());
        let many: Vec<String> = (0..=MAX_FILES).map(|i| format!("f{i}.csv")).collect();
        assert!(validate_names(many.iter().map(String::as_str)).is_err());
        assert!(validate_names(["a.csv", "b.csv"]).is_ok());
    }
}

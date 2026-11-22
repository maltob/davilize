//! `manifest.json`: the list of files in a publish with sizes and SHA-256 digests,
//! plus the report name and version and the generation time (UTC, RFC 3339).

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::PublishError;
use crate::publish::FileToPublish;

/// File name of the manifest inside a publish.
pub const MANIFEST_NAME: &str = "manifest.json";
/// Value of the `format` field, so readers can recognise the document.
pub const MANIFEST_FORMAT: &str = "davilize-publish-manifest";
/// Version of the manifest layout. Bump when fields change meaning.
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ManifestReport {
    pub name: String,
    pub version: u32,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: &'static str,
    pub format_version: u32,
    pub report: ManifestReport,
    pub generated_at: String,
    pub files: Vec<ManifestFile>,
}

/// Builds the manifest bytes (pretty JSON, trailing newline) for `files`.
/// An entry named [`MANIFEST_NAME`] is skipped, so the manifest never lists itself.
pub fn build_manifest(
    report_name: &str,
    report_version: u32,
    generated_at_unix: u64,
    files: &[FileToPublish],
) -> Result<Vec<u8>, PublishError> {
    let mut entries = Vec::new();
    for file in files.iter().filter(|f| f.name != MANIFEST_NAME) {
        let (size, sha256) = file.content.size_and_sha256()?;
        entries.push(ManifestFile {
            name: file.name.clone(),
            size,
            sha256,
        });
    }
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        format_version: MANIFEST_FORMAT_VERSION,
        report: ManifestReport {
            name: report_name.to_string(),
            version: report_version,
        },
        generated_at: rfc3339_utc(generated_at_unix),
        files: entries,
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| PublishError::Io(format!("cannot encode the manifest: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

/// Streams a file through SHA-256. Returns its size and digest.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<(u64, String)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((size, hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Formats Unix seconds as an RFC 3339 UTC timestamp, e.g. `2023-11-14T22:13:20Z`.
pub fn rfc3339_utc(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let secs_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        secs_of_day % 3600 / 60,
        secs_of_day % 60
    )
}

/// Converts days since 1970-01-01 to a (year, month, day) civil date
/// (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::Content;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        // 2024-02-29 is a leap day.
        assert_eq!(rfc3339_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn manifest_lists_sizes_and_digests_and_skips_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.csv");
        std::fs::write(&path, b"abc").unwrap();
        let files = vec![
            FileToPublish {
                name: "index.html".into(),
                content: Content::Bytes(b"<html>".to_vec()),
            },
            FileToPublish {
                name: "data.csv".into(),
                content: Content::Path(path),
            },
            FileToPublish {
                name: MANIFEST_NAME.into(),
                content: Content::Bytes(b"old".to_vec()),
            },
        ];
        let bytes = build_manifest("Sample sales", 1, 1_700_000_000, &files).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["format"], MANIFEST_FORMAT);
        assert_eq!(value["report"]["name"], "Sample sales");
        assert_eq!(value["report"]["version"], 1);
        assert_eq!(value["generated_at"], "2023-11-14T22:13:20Z");
        let listed = value["files"].as_array().unwrap();
        assert_eq!(listed.len(), 2, "manifest must not list itself");
        assert_eq!(listed[1]["name"], "data.csv");
        assert_eq!(listed[1]["size"], 3);
        assert_eq!(
            listed[1]["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(listed[0]["size"], 6);
    }
}

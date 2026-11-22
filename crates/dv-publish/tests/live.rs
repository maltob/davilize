//! Opt-in tests against real storage. Each test is skipped (it passes and prints a
//! notice) unless its environment variable is set, so the default `cargo test` never
//! needs credentials or network.
//!
//! * `DAVILIZE_LIVE_S3_TARGET=s3://bucket/some/prefix` plus the AWS_* variables
//!   (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION, and AWS_ENDPOINT_URL for
//!   MinIO or another S3-compatible store). Writes two small objects under the prefix.
//! * `DAVILIZE_LIVE_SFTP_TARGET=sftp://user@host[:port]/writable/path` plus
//!   DAVILIZE_SFTP_KEY (and DAVILIZE_SFTP_KEY_PASSPHRASE if encrypted) or
//!   DAVILIZE_SFTP_PASSWORD. The host key must already be in known_hosts, or set
//!   DAVILIZE_LIVE_SFTP_TRUST=1 to record it on first use. Writes two small files.

use dv_publish::{
    Content, Env, FileToPublish, INDEX_NAME, PublishOptions, PublishTarget, publish,
    publish_with_options,
};

fn payload(tag: &str) -> Vec<FileToPublish> {
    vec![
        FileToPublish {
            name: "davilize-live-check.txt".into(),
            content: Content::Bytes(format!("live check {tag}\n").into_bytes()),
        },
        FileToPublish {
            name: INDEX_NAME.into(),
            content: Content::Bytes(format!("<!doctype html><p>live check {tag}</p>").into_bytes()),
        },
    ]
}

#[test]
fn live_s3_publish_round_trip() {
    let Ok(raw) = std::env::var("DAVILIZE_LIVE_S3_TARGET") else {
        eprintln!("skipped: DAVILIZE_LIVE_S3_TARGET is not set");
        return;
    };
    let target = PublishTarget::parse(&raw).expect("DAVILIZE_LIVE_S3_TARGET parses");
    let report = publish(&payload("s3"), &target, &Env::from_process())
        .expect("publish to the live S3 target");
    assert_eq!(report.uploaded.len(), 2);
    assert_eq!(report.uploaded.last().unwrap().name, INDEX_NAME);
}

#[test]
fn live_sftp_publish_round_trip() {
    let Ok(raw) = std::env::var("DAVILIZE_LIVE_SFTP_TARGET") else {
        eprintln!("skipped: DAVILIZE_LIVE_SFTP_TARGET is not set");
        return;
    };
    let target = PublishTarget::parse(&raw).expect("DAVILIZE_LIVE_SFTP_TARGET parses");
    let options = PublishOptions {
        trust_new_host_key: std::env::var("DAVILIZE_LIVE_SFTP_TRUST").is_ok_and(|v| v == "1"),
    };
    let report = publish_with_options(&payload("sftp"), &target, &Env::from_process(), &options)
        .expect("publish to the live SFTP target");
    assert_eq!(report.uploaded.len(), 2);
    assert_eq!(report.uploaded.last().unwrap().name, INDEX_NAME);
}

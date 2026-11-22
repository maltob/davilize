//! End-to-end publish to a local folder, plus the failure cases that must not touch
//! the network: traversal names, missing or inconsistent credentials, and a disabled
//! SFTP feature. Real S3 and SFTP runs are in tests/live.rs.

use dv_publish::{
    Content, Env, FileToPublish, INDEX_NAME, MANIFEST_NAME, PublishError, PublishTarget,
    build_manifest, publish,
};

fn text(name: &str, body: &str) -> FileToPublish {
    FileToPublish {
        name: name.to_string(),
        content: Content::Bytes(body.as_bytes().to_vec()),
    }
}

fn no_env() -> Env {
    Env::from_pairs::<Vec<(String, String)>, String, String>(Vec::new())
}

#[test]
fn local_folder_publish_writes_every_file_and_leaves_no_temporaries() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("published");
    let data_path = base.path().join("sales.csv");
    std::fs::write(&data_path, b"region,total\nnorth,10\n").unwrap();

    let mut files = vec![
        FileToPublish {
            name: "sales.csv".into(),
            content: Content::Path(data_path),
        },
        text(INDEX_NAME, "<!doctype html><p>report</p>"),
    ];
    let manifest = build_manifest("Sample sales", 1, 1_700_000_000, &files).unwrap();
    files.push(FileToPublish {
        name: MANIFEST_NAME.into(),
        content: Content::Bytes(manifest.clone()),
    });

    let target = PublishTarget::parse(root.to_str().unwrap()).unwrap();
    let report = publish(&files, &target, &no_env()).expect("publish succeeds");

    let order: Vec<&str> = report.uploaded.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        order,
        ["sales.csv", MANIFEST_NAME, INDEX_NAME],
        "index.html must go last"
    );
    assert_eq!(report.uploaded[0].bytes, 22);

    assert_eq!(
        std::fs::read(root.join("sales.csv")).unwrap(),
        b"region,total\nnorth,10\n"
    );
    assert_eq!(std::fs::read(root.join(MANIFEST_NAME)).unwrap(), manifest);
    assert_eq!(
        std::fs::read_to_string(root.join(INDEX_NAME)).unwrap(),
        "<!doctype html><p>report</p>"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".part-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
}

#[test]
fn republishing_replaces_files_in_place() {
    let base = tempfile::tempdir().unwrap();
    let target = PublishTarget::Local {
        root: base.path().join("site"),
    };
    publish(&[text(INDEX_NAME, "first")], &target, &no_env()).unwrap();
    publish(&[text(INDEX_NAME, "second")], &target, &no_env()).unwrap();
    assert_eq!(
        std::fs::read_to_string(base.path().join("site").join(INDEX_NAME)).unwrap(),
        "second"
    );
}

#[test]
fn traversal_names_are_refused_and_nothing_is_written_outside() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("site");
    let target = PublishTarget::Local { root: root.clone() };
    for bad in [
        "../escape.html",
        "/abs.html",
        "C:\\evil.html",
        "a/../../escape.html",
    ] {
        let files = [text(INDEX_NAME, "ok"), text(bad, "pwned")];
        let err = publish(&files, &target, &no_env()).unwrap_err();
        assert!(
            matches!(err, PublishError::InvalidName { .. }),
            "{bad}: {err}"
        );
    }
    assert!(!base.path().join("escape.html").exists());
    assert!(!root.exists() || std::fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn credentials_missing_for_s3_are_a_config_error_and_never_echo_the_key() {
    let target = PublishTarget::parse("s3://reports-bucket/q3").unwrap();
    let env = Env::from_pairs([("AWS_ACCESS_KEY_ID", "AKIAFAKEKEYVALUE")]);
    let err = publish(&[text(INDEX_NAME, "x")], &target, &env).unwrap_err();
    let message = err.to_string();
    assert!(matches!(err, PublishError::Config(_)), "{message}");
    assert!(message.contains("must be set together"), "{message}");
    assert!(!message.contains("AKIAFAKEKEYVALUE"), "{message}");
}

#[test]
fn an_empty_publish_still_needs_valid_inputs_and_a_folder() {
    let base = tempfile::tempdir().unwrap();
    let target = PublishTarget::Local {
        root: base.path().join("empty"),
    };
    let report = publish(&[], &target, &no_env()).unwrap();
    assert!(report.uploaded.is_empty());
    assert!(base.path().join("empty").is_dir());
}

#[test]
fn sftp_without_a_user_is_a_config_error_before_any_connection() {
    // The user name check runs before the network is touched.
    let target = PublishTarget::parse("sftp://example.invalid/srv").unwrap();
    let err = publish(&[text(INDEX_NAME, "x")], &target, &no_env()).unwrap_err();
    assert!(matches!(err, PublishError::Config(_)), "{err}");
    assert!(err.to_string().contains("DAVILIZE_SFTP_USER"), "{err}");
}

//! Smoke test: `davilize refresh` over examples/sample_joined.dvz, whose dataset joins
//! examples/regions.csv onto examples/sample.csv. Outputs go to a temporary directory.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sample_joined() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sample_joined.dvz")
        .canonicalize()
        .expect("examples/sample_joined.dvz exists")
}

#[test]
fn refresh_runs_the_joined_dataset_and_exposes_the_joined_column() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_davilize"))
        .arg("refresh")
        .arg(sample_joined())
        .arg("--out")
        .arg(dir.path())
        .arg("--format")
        .arg("csv")
        .output()
        .expect("davilize runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let csv = std::fs::read_to_string(dir.path().join("manager_revenue.csv"))
        .expect("manager_revenue.csv is written");
    let header = csv.lines().next().unwrap_or_default();
    assert!(header.contains("manager"), "{header}");
    assert!(header.contains("total_revenue"), "{header}");
    // One row per manager: the left join keeps every region in sales.
    for manager in ["Ann", "Bo", "Cy", "Dee"] {
        assert!(csv.contains(manager), "missing {manager} in:\n{csv}");
    }
}

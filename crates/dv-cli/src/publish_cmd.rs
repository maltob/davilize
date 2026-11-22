//! `davilize publish`: refresh a report into a temporary folder, add `manifest.json`
//! and upload everything to a publish target. `index.html` is always included and
//! uploaded last.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use dv_publish::{
    Content, Env, FileToPublish, INDEX_NAME, MANIFEST_NAME, PublishError, PublishOptions,
    PublishTarget, build_manifest, publish_with_options, validate_files,
};
use serde_json::json;

use crate::pipeline;
use crate::{CliError, CliResult, DataOutput, EXIT_INPUT, EXIT_PUBLISH, Log, output_error};

/// Formats that can be published. `html` is always included (it is the index page).
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PublishFormat {
    Html,
    Xlsx,
    Png,
    Pdf,
    Csv,
    Parquet,
}

impl PublishFormat {
    fn data_output(self) -> DataOutput {
        match self {
            PublishFormat::Html => DataOutput::Html,
            PublishFormat::Xlsx => DataOutput::Xlsx,
            PublishFormat::Png => DataOutput::Png,
            PublishFormat::Pdf => DataOutput::Pdf,
            PublishFormat::Csv => DataOutput::Csv,
            PublishFormat::Parquet => DataOutput::Parquet,
        }
    }
}

/// Maps a publish error to an exit code: a bad target or file name is an input
/// problem (3); everything else (credentials, network, backend) is EXIT_PUBLISH (6).
pub fn publish_error(err: PublishError) -> CliError {
    let code = match err {
        PublishError::InvalidTarget(_) | PublishError::InvalidName { .. } => EXIT_INPUT,
        _ => EXIT_PUBLISH,
    };
    CliError {
        code,
        message: err.to_string(),
    }
}

/// Inputs of one `publish` run, grouped so the call stays readable.
pub struct PublishRequest<'a> {
    pub report: &'a Path,
    pub target: &'a str,
    pub formats: &'a [PublishFormat],
    pub vendor_dir: Option<&'a Path>,
    pub dry_run: bool,
    pub trust_new_host_key: bool,
    pub params: Vec<(String, String)>,
}

pub fn publish_report(request: PublishRequest, log: &Log) -> CliResult<()> {
    let PublishRequest {
        report,
        target,
        formats,
        vendor_dir,
        dry_run,
        trust_new_host_key,
        params,
    } = request;
    let target = PublishTarget::parse(target).map_err(publish_error)?;

    let mut outputs = vec![DataOutput::Html];
    for format in formats {
        let output = format.data_output();
        if !outputs.contains(&output) {
            outputs.push(output);
        }
    }
    let work = tempfile::Builder::new()
        .prefix("davilize-publish-")
        .tempdir()
        .map_err(|e| output_error(format!("cannot create a temporary folder: {e}")))?;
    let options = pipeline::Options {
        formats: outputs,
        vendor_dir: vendor_dir.map(PathBuf::from),
        html_name: Some(INDEX_NAME.to_string()),
        params,
    };
    let refreshed = pipeline::run(report, work.path(), &options, log)?;

    let mut files = collect_files(work.path())?;
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let manifest = build_manifest(
        &refreshed.report_name,
        refreshed.report_version,
        generated_at,
        &files,
    )
    .map_err(publish_error)?;
    files.push(FileToPublish {
        name: MANIFEST_NAME.to_string(),
        content: Content::Bytes(manifest),
    });
    validate_files(&files).map_err(publish_error)?;

    if dry_run {
        print_plan(&files, &target.describe())?;
        return Ok(());
    }

    let env = Env::from_process();
    let options = PublishOptions { trust_new_host_key };
    let report = publish_with_options(&files, &target, &env, &options).map_err(publish_error)?;
    for file in &report.uploaded {
        log.info(
            &format!("uploaded {} ({} bytes)", file.name, file.bytes),
            json!({"file": file.name, "bytes": file.bytes}),
        );
    }
    log.info(
        &format!(
            "published {} files to {}",
            report.uploaded.len(),
            report.target
        ),
        json!({"target": report.target, "files": report.uploaded.len(), "status": "ok"}),
    );
    Ok(())
}

/// Regular files directly inside `dir`, sorted by name. The refresh writes a flat folder.
fn collect_files(dir: &Path) -> CliResult<Vec<FileToPublish>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| output_error(format!("cannot read '{}': {e}", dir.display())))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(output_error)?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().into_string().map_err(|_| CliError {
            code: EXIT_INPUT,
            message: "a generated file name is not valid UTF-8".to_string(),
        })?;
        files.push(FileToPublish {
            name,
            content: Content::Path(path),
        });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Prints the files in upload order, with sizes and digests, without connecting.
fn print_plan(files: &[FileToPublish], target: &str) -> CliResult<()> {
    let rank = |name: &str| match name {
        INDEX_NAME => 2,
        MANIFEST_NAME => 1,
        _ => 0,
    };
    let mut ordered: Vec<&FileToPublish> = files.iter().collect();
    ordered.sort_by_key(|f| rank(&f.name));
    println!(
        "Dry run: {} files would be published to {target}",
        ordered.len()
    );
    for file in ordered {
        let (size, sha256) = file.content.size_and_sha256().map_err(publish_error)?;
        println!("  {:>12}  {}  sha256:{}", size, file.name, &sha256[..16]);
    }
    Ok(())
}

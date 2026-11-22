//! `davilize serve`: static server for a published folder. With `--refresh`, the
//! report is re-run every N seconds (or on `POST /refresh`) into a new folder
//! generation, which replaces the served one in a single step.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dv_publish::serve::{self, ServeOptions, Site};
use serde_json::json;

use crate::pipeline;
use crate::{CliError, CliResult, EXIT_INPUT, EXIT_PUBLISH, Log, output_error};

/// Environment variable holding the bearer token for `POST /refresh`.
pub const REFRESH_TOKEN_ENV: &str = "DAVILIZE_REFRESH_TOKEN";

/// Background refresh settings.
pub struct RefreshPlan {
    pub report: PathBuf,
    pub every: Duration,
    pub options: pipeline::Options,
}

pub fn serve(
    dir: &Path,
    port: u16,
    bind: Option<IpAddr>,
    refresh: Option<RefreshPlan>,
    log: &Log,
) -> CliResult<()> {
    // Loopback unless the user asked for another address explicitly.
    let ip = bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    if !ip.is_loopback() {
        log.warn(
            "listening on a non-loopback address: the server has no TLS, so put a reverse proxy in front of it for remote access",
            json!({"bind": ip.to_string()}),
        );
    }

    let (site, options) = match refresh {
        None => {
            if !dir.is_dir() {
                return Err(CliError {
                    code: EXIT_INPUT,
                    message: format!("'{}' is not a folder", dir.display()),
                });
            }
            (Site::new(dir.to_path_buf()), ServeOptions::default())
        }
        Some(plan) => {
            std::fs::create_dir_all(dir)
                .map_err(|e| output_error(format!("cannot create '{}': {e}", dir.display())))?;
            remove_old_generations(dir);
            let first = new_generation(dir)?;
            if let Err(err) = pipeline::run(&plan.report, &first, &plan.options, log) {
                let _ = std::fs::remove_dir_all(&first);
                return Err(err);
            }
            let site = Site::new(first);
            let (trigger, requests) = mpsc::channel::<()>();
            let token = std::env::var(REFRESH_TOKEN_ENV)
                .ok()
                .filter(|t| !t.is_empty());
            let worker_site = site.clone();
            let worker_dir = dir.to_path_buf();
            let worker_log = *log;
            std::thread::spawn(move || {
                refresh_loop(requests, plan, &worker_dir, &worker_site, worker_log)
            });
            let options = ServeOptions {
                refresh_token: token,
                refresh_trigger: Some(trigger),
            };
            (site, options)
        }
    };

    let listener = TcpListener::bind(SocketAddr::new(ip, port)).map_err(|e| CliError {
        code: EXIT_PUBLISH,
        message: format!("cannot listen on {ip}:{port}: {e}"),
    })?;
    let addr = listener
        .local_addr()
        .map_err(|e| output_error(format!("cannot read the listening address: {e}")))?;
    log.info(
        &format!("serving '{}' at http://{addr}/", dir.display()),
        json!({"url": format!("http://{addr}/"), "folder": dir.display().to_string()}),
    );
    serve::run(listener, site, options).map_err(|e| CliError {
        code: EXIT_PUBLISH,
        message: format!("the server stopped: {e}"),
    })
}

/// Waits for the interval or a refresh request, then builds a new generation and
/// swaps it in. A failed refresh leaves the previous generation in place.
fn refresh_loop(requests: Receiver<()>, plan: RefreshPlan, dir: &Path, site: &Site, log: Log) {
    loop {
        match requests.recv_timeout(plan.every) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let next = match new_generation(dir) {
            Ok(path) => path,
            Err(err) => {
                log.error(&format!("refresh skipped: {}", err.message));
                continue;
            }
        };
        match pipeline::run(&plan.report, &next, &plan.options, &log) {
            Ok(_) => {
                let old = site.swap(next.clone());
                let _ = std::fs::remove_dir_all(&old);
                log.info(
                    &format!("now serving refreshed output from '{}'", next.display()),
                    json!({"generation": next.display().to_string()}),
                );
            }
            Err(err) => {
                let _ = std::fs::remove_dir_all(&next);
                log.error(&format!(
                    "refresh failed; still serving the previous output: {}",
                    err.message
                ));
            }
        }
    }
}

/// A new, empty, hidden folder inside `dir` for one generation of output.
/// Hidden names are never reachable by URL, so a generation is only seen as a root.
fn new_generation(dir: &Path) -> CliResult<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!(
        ".gen-{nanos:x}-{}",
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path)
        .map_err(|e| output_error(format!("cannot create '{}': {e}", path.display())))?;
    Ok(path)
}

/// Removes generation folders left by an earlier run that stopped mid-refresh.
/// Only names starting with `.gen-` are touched.
fn remove_old_generations(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(".gen-") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

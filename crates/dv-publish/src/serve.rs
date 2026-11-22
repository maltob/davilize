//! Static server for a published folder.
//!
//! * Serves files under a [`Site`] root. The root can be swapped atomically, so
//!   a refresher can publish a new complete folder while readers never see a mix.
//! * Paths are decoded and checked segment by segment. Dot-segments, hidden
//!   names (leading `.`), backslashes, colons, control characters and encoded
//!   slashes are refused, and the resolved file must still lie inside the root
//!   after symlinks are followed.
//! * Security headers on every response. The report HTML gets a CSP that allows
//!   the pinned CDN and inline scripts (the page needs both) and `unsafe-eval`
//!   (vega's expression code generator uses `Function`).
//! * `POST /refresh` triggers a refresh when a bearer token is configured. Without
//!   a token, or without a refresher, the route answers 404.
//!
//! The server binds whatever listener it is given. The CLI decides the address.

use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use percent_encoding::percent_decode_str;

/// Content-Security-Policy for HTML pages. Scripts come from this origin, inline
/// blocks, or the jsDelivr CDN used by the default (non-offline) report.
const HTML_CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline' 'unsafe-eval' https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// Content-Security-Policy for everything else. Nothing executes: a directly opened
/// SVG cannot run script.
const ASSET_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'";

/// The folder being served. Clones share the same root.
#[derive(Clone)]
pub struct Site {
    root: Arc<RwLock<PathBuf>>,
}

impl Site {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Site {
            root: Arc::new(RwLock::new(root.into())),
        }
    }

    /// The folder currently served.
    pub fn root(&self) -> PathBuf {
        match self.root.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Serves `new_root` from now on and returns the previous root. The caller
    /// deletes the old folder once in-flight requests have had time to finish.
    pub fn swap(&self, new_root: PathBuf) -> PathBuf {
        let mut guard = match self.root.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::mem::replace(&mut *guard, new_root)
    }
}

/// Optional behaviour for [`run`].
#[derive(Default)]
pub struct ServeOptions {
    /// Bearer token that authorises `POST /refresh`. `None` disables the route.
    pub refresh_token: Option<String>,
    /// Receives one message per accepted `POST /refresh`.
    pub refresh_trigger: Option<Sender<()>>,
}

struct App {
    site: Site,
    refresh_token: Option<String>,
    refresh_trigger: Option<Sender<()>>,
}

/// Serves until the listener fails. Blocks the calling thread.
pub fn run(listener: TcpListener, site: Site, options: ServeOptions) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let app = Arc::new(App {
        site,
        refresh_token: options.refresh_token.filter(|t| !t.is_empty()),
        refresh_trigger: options.refresh_trigger,
    });
    let router = Router::new().fallback(handle).with_state(app);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)?;
        axum::serve(listener, router).await
    })
}

async fn handle(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if uri.path() == "/refresh" && method == Method::POST {
        return refresh(&app, &headers);
    }
    let head = method == Method::HEAD;
    if method != Method::GET && !head {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed", head);
    }
    let Some(parts) = request_components(uri.path()) else {
        return plain(StatusCode::NOT_FOUND, "not found", head);
    };
    match serve_file(&app.site.root(), &parts, head).await {
        Some(response) => response,
        None => plain(StatusCode::NOT_FOUND, "not found", head),
    }
}

fn refresh(app: &App, headers: &HeaderMap) -> Response {
    let (Some(token), Some(trigger)) = (&app.refresh_token, &app.refresh_trigger) else {
        return plain(StatusCode::NOT_FOUND, "not found", false);
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorised = presented.is_some_and(|p| constant_time_eq(p.as_bytes(), token.as_bytes()));
    if !authorised {
        let mut response = plain(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token",
            false,
        );
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    match trigger.send(()) {
        Ok(()) => plain(StatusCode::ACCEPTED, "refresh queued", false),
        Err(_) => plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "the refresher has stopped",
            false,
        ),
    }
}

/// Splits a raw (still percent-encoded) request path into safe path components.
/// Returns `None` when any component is unsafe. The empty vector means the root.
pub(crate) fn request_components(raw_path: &str) -> Option<Vec<String>> {
    let rest = raw_path.strip_prefix('/')?;
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let pieces: Vec<&str> = rest.split('/').collect();
    let last = pieces.len() - 1;
    let mut parts = Vec::with_capacity(pieces.len());
    for (index, piece) in pieces.iter().enumerate() {
        if piece.is_empty() {
            // A single trailing slash is fine (directory request); anything else is not.
            if index == last {
                continue;
            }
            return None;
        }
        // Decode one level only. An encoded slash becomes '/' inside a component, which
        // safe_component rejects, so %2F cannot smuggle in a separator.
        let decoded = percent_decode_str(piece).decode_utf8().ok()?.into_owned();
        if !safe_component(&decoded) {
            return None;
        }
        parts.push(decoded);
    }
    Some(parts)
}

fn safe_component(component: &str) -> bool {
    !component.is_empty()
        && !component.starts_with('.')
        && !component.contains(['/', '\\', ':'])
        && !component.chars().any(char::is_control)
}

/// Reads the file for `parts` under `root`, or `None` when it does not exist or
/// resolves outside the root.
async fn serve_file(root: &Path, parts: &[String], head: bool) -> Option<Response> {
    let root = tokio::fs::canonicalize(root).await.ok()?;
    let mut path = root.clone();
    path.extend(parts.iter());
    // canonicalize follows symlinks, so a link pointing outside the root is caught here.
    let mut path = tokio::fs::canonicalize(&path).await.ok()?;
    if !path.starts_with(&root) {
        return None;
    }
    let mut meta = tokio::fs::metadata(&path).await.ok()?;
    if meta.is_dir() {
        path = tokio::fs::canonicalize(path.join("index.html"))
            .await
            .ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        meta = tokio::fs::metadata(&path).await.ok()?;
    }
    if !meta.is_file() {
        return None;
    }
    let is_html = matches!(extension(&path).as_str(), "html" | "htm");
    let body = if head {
        Body::empty()
    } else {
        Body::from(tokio::fs::read(&path).await.ok()?)
    };
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&path)),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(meta.len()));
    apply_security_headers(headers, if is_html { HTML_CSP } else { ASSET_CSP });
    Some(response)
}

fn plain(status: StatusCode, message: &'static str, head: bool) -> Response {
    let body = if head {
        Body::empty()
    } else {
        Body::from(message)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(message.len()));
    apply_security_headers(headers, ASSET_CSP);
    response
}

fn apply_security_headers(headers: &mut HeaderMap, csp: &'static str) {
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(csp),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Content type by extension. Text types carry an explicit UTF-8 charset.
pub(crate) fn content_type(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "parquet" => "application/vnd.apache.parquet",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Compares secrets without leaking the position of the first mismatch.
/// The length is not hidden, which is acceptable for a fixed-size token.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_encoded_safe_paths_are_accepted() {
        assert_eq!(request_components("/"), Some(vec![]));
        assert_eq!(
            request_components("/index.html"),
            Some(vec!["index.html".to_string()])
        );
        assert_eq!(
            request_components("/charts/revenue%20by%20region.svg"),
            Some(vec![
                "charts".to_string(),
                "revenue by region.svg".to_string()
            ])
        );
        assert_eq!(
            request_components("/charts/"),
            Some(vec!["charts".to_string()])
        );
    }

    #[test]
    fn traversal_in_every_spelling_is_refused() {
        for raw in [
            "/../secret",
            "/%2e%2e/secret",
            "/%2E%2E/secret",
            "/..%2fsecret",
            "/..%2Fsecret",
            "/%2e%2e%2fsecret",
            "/a/../../secret",
            "/a/%2e%2e/%2e%2e/secret",
            "/..\\secret",
            "/%2e%2e%5csecret",
            "/a//b",
            "/./index.html",
            "/.hidden",
            "/%2ehidden",
            "/C:secret",
            "/%00",
            "/bad%ff",
            "*",
        ] {
            assert_eq!(request_components(raw), None, "{raw} must be refused");
        }
    }

    #[test]
    fn content_types_cover_report_outputs() {
        let ct = |name: &str| content_type(Path::new(name));
        assert_eq!(ct("index.html"), "text/html; charset=utf-8");
        assert_eq!(ct("A.PNG"), "image/png");
        assert_eq!(ct("data.parquet"), "application/vnd.apache.parquet");
        assert_eq!(
            ct("sample.xlsx"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(ct("unknown.bin"), "application/octet-stream");
    }

    #[test]
    fn constant_time_compare_is_exact() {
        assert!(constant_time_eq(b"token-123", b"token-123"));
        assert!(!constant_time_eq(b"token-123", b"token-124"));
        assert!(!constant_time_eq(b"token", b"token-123"));
    }

    #[test]
    fn site_swap_returns_the_previous_root() {
        let site = Site::new("gen-1");
        let old = site.swap(PathBuf::from("gen-2"));
        assert_eq!(old, PathBuf::from("gen-1"));
        assert_eq!(site.root(), PathBuf::from("gen-2"));
    }
}

//! Integration tests for the static server. Requests are written by hand over a
//! TCP socket so that encoded traversal spellings reach the server unchanged.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use dv_publish::serve::{ServeOptions, Site, run};

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn request(addr: SocketAddr, method: &str, path: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read reply");
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header terminator");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = raw[split + 4..].to_vec();
    let mut lines = head.lines();
    let status_line = lines.next().expect("status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status code");
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    Reply {
        status,
        headers,
        body,
    }
}

fn start(site: Site, options: ServeOptions) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let _ = run(listener, site, options);
    });
    addr
}

/// A base folder with `site/` (served) and a secret next to it, outside the root.
struct Fixture {
    _dir: tempfile::TempDir,
    site: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    let site = base.join("site");
    std::fs::create_dir_all(site.join("charts")).unwrap();
    std::fs::write(site.join("index.html"), b"<!doctype html><title>r</title>").unwrap();
    std::fs::write(site.join("data.csv"), b"a,b\n1,2\n").unwrap();
    std::fs::write(
        site.join("charts/chart.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    )
    .unwrap();
    std::fs::write(site.join("sample.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    std::fs::write(site.join(".hidden"), b"do not serve").unwrap();
    std::fs::write(base.join("outside.txt"), b"TOP SECRET").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("outside.txt"), site.join("escape.txt")).unwrap();
    Fixture { _dir: dir, site }
}

#[test]
fn serves_files_with_content_types_and_security_headers() {
    let fx = fixture();
    let addr = start(Site::new(&fx.site), ServeOptions::default());

    let index = request(addr, "GET", "/");
    assert_eq!(index.status, 200);
    assert_eq!(
        index.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(index.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(index.header("x-frame-options"), Some("DENY"));
    assert_eq!(index.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(index.header("cache-control"), Some("no-cache"));
    let csp = index.header("content-security-policy").unwrap();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(csp.contains("https://cdn.jsdelivr.net"), "{csp}");
    assert!(csp.contains("'unsafe-inline'"), "{csp}");
    assert_eq!(index.body, b"<!doctype html><title>r</title>");

    let csv = request(addr, "GET", "/data.csv");
    assert_eq!(csv.status, 200);
    assert_eq!(csv.header("content-type"), Some("text/csv; charset=utf-8"));
    let asset_csp = csv.header("content-security-policy").unwrap();
    assert!(!asset_csp.contains("script-src"), "{asset_csp}");

    let svg = request(addr, "GET", "/charts/chart.svg");
    assert_eq!(svg.status, 200);
    assert_eq!(svg.header("content-type"), Some("image/svg+xml"));

    let png = request(addr, "GET", "/sample.png");
    assert_eq!(png.header("content-type"), Some("image/png"));
    assert_eq!(png.body, b"\x89PNG\r\n\x1a\n");
    assert_eq!(png.header("content-length"), Some("8"));
}

#[test]
fn directory_requests_use_index_html_and_head_has_no_body() {
    let fx = fixture();
    let addr = start(Site::new(&fx.site), ServeOptions::default());

    let root_head = request(addr, "HEAD", "/index.html");
    assert_eq!(root_head.status, 200);
    assert!(root_head.body.is_empty());
    assert_eq!(root_head.header("content-length"), Some("31"));

    // A folder without an index.html is not listed.
    assert_eq!(request(addr, "GET", "/charts/").status, 404);
    assert_eq!(request(addr, "GET", "/charts").status, 404);
}

#[test]
fn traversal_attempts_in_every_encoding_get_404() {
    let fx = fixture();
    let addr = start(Site::new(&fx.site), ServeOptions::default());
    for path in [
        "/../outside.txt",
        "/%2e%2e/outside.txt",
        "/%2E%2E/outside.txt",
        "/..%2foutside.txt",
        "/..%2Foutside.txt",
        "/%2e%2e%2foutside.txt",
        "/%2e%2e%5coutside.txt",
        "/..\\outside.txt",
        "/charts/../../outside.txt",
        "/charts/%2e%2e/%2e%2e/outside.txt",
        "/%252e%252e/outside.txt",
        "/.hidden",
        "/%2ehidden",
        "/missing.txt",
        "/data.csv%00.txt",
    ] {
        let reply = request(addr, "GET", path);
        assert!(
            matches!(reply.status, 400 | 404),
            "{path} must not be served, got {}",
            reply.status
        );
        assert!(
            !String::from_utf8_lossy(&reply.body).contains("TOP SECRET"),
            "{path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinks_that_point_outside_the_root_are_not_followed() {
    let fx = fixture();
    let addr = start(Site::new(&fx.site), ServeOptions::default());
    let reply = request(addr, "GET", "/escape.txt");
    assert_eq!(reply.status, 404);
    assert!(!String::from_utf8_lossy(&reply.body).contains("TOP SECRET"));
}

#[test]
fn other_methods_are_refused_and_refresh_is_off_by_default() {
    let fx = fixture();
    let addr = start(Site::new(&fx.site), ServeOptions::default());
    let post_file = request(addr, "POST", "/data.csv");
    assert_eq!(post_file.status, 405);
    assert_eq!(request(addr, "PUT", "/index.html").status, 405);
    // Without a configured token the refresh route does not exist.
    assert_eq!(request(addr, "POST", "/refresh").status, 404);
}

#[test]
fn refresh_requires_the_bearer_token_and_then_triggers_once() {
    let fx = fixture();
    let (trigger, requests) = mpsc::channel::<()>();
    let addr = start(
        Site::new(&fx.site),
        ServeOptions {
            refresh_token: Some("token-abc-123".into()),
            refresh_trigger: Some(trigger),
        },
    );
    let anonymous = request(addr, "POST", "/refresh");
    assert_eq!(anonymous.status, 401);
    assert_eq!(anonymous.header("www-authenticate"), Some("Bearer"));
    assert!(
        requests.try_recv().is_err(),
        "unauthenticated request must not trigger"
    );

    // Wrong token and GET are both refused.
    let wrong = raw_with_auth(addr, "POST", "/refresh", "Bearer token-abc-124");
    assert_eq!(wrong.status, 401);
    assert_eq!(request(addr, "GET", "/refresh").status, 404);

    let good = raw_with_auth(addr, "POST", "/refresh", "Bearer token-abc-123");
    assert_eq!(good.status, 202);
    requests
        .recv_timeout(Duration::from_secs(5))
        .expect("refresh trigger received");
    assert!(
        requests.try_recv().is_err(),
        "exactly one trigger per request"
    );
}

fn raw_with_auth(addr: SocketAddr, method: &str, path: &str, auth: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {auth}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Reply {
        status,
        headers: Vec::new(),
        body: raw[split + 4..].to_vec(),
    }
}

#[test]
fn swapping_the_root_changes_what_is_served_without_a_restart() {
    let fx = fixture();
    let site = Site::new(&fx.site);
    let addr = start(site.clone(), ServeOptions::default());
    assert_eq!(request(addr, "GET", "/data.csv").status, 200);

    let next = fx.site.parent().unwrap().join("next");
    std::fs::create_dir_all(&next).unwrap();
    std::fs::write(next.join("index.html"), b"<p>generation two</p>").unwrap();
    let old = site.swap(next.clone());
    assert_eq!(old, fx.site);
    assert_eq!(request(addr, "GET", "/").body, b"<p>generation two</p>");
    // Files from the old generation are gone from the served root.
    assert_eq!(request(addr, "GET", "/data.csv").status, 404);
}

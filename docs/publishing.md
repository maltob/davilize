# Publishing and serving reports

Davilize publishes a refreshed report to a folder, an S3 bucket, or an SFTP server, and
serves a published folder over HTTP. This page covers the targets, credentials, host
key checks, the file manifest, the server, the exit codes, and what has been tested.

## Commands

```
davilize publish <report.dvz> --target <url> [--format html,xlsx,png,pdf,csv,parquet]
                 [--offline --vendor-dir DIR] [--dry-run] [--trust-new-host-key]

davilize serve <dir> [--port 8080] [--bind 127.0.0.1]
               [--refresh <report.dvz> [--every SECONDS] [--format ...] [--offline --vendor-dir DIR]]
```

`publish` runs the same refresh pipeline as `refresh` into a temporary folder, adds
`manifest.json`, and uploads the folder. `index.html` is always included (`html` is
implied by `--format`). It is uploaded last.

`--dry-run` runs the refresh and prints the files with sizes and short digests, in
upload order. It does not connect to the target.

## Targets

| Target | Example | Notes |
|---|---|---|
| Local folder | `--target ./published`, `--target file:///srv/reports` | Created if missing. Files are written to a temporary name and renamed into place. |
| S3 or S3-compatible | `--target s3://bucket/team/q3` | Region and endpoint come from the environment (below). Objects are written directly; S3 applies each PUT atomically. |
| SFTP | `--target sftp://deploy@example.com:2222/var/www/reports` | Pure-Rust SSH client (russh). No external `ssh` program is needed, so it works the same on Windows, macOS and Linux. Port defaults to 22. |

A target string without `://` is a local path. URLs must not contain a password, a
query string or a fragment. A `..` segment in a URL is an error.

Azure Blob, Google Cloud Storage and WebDAV are not implemented. OpenDAL supports
them, but they were not in scope for this change.

## Credentials

Credentials are read from the environment only. They never appear in `.dvz` files,
in URLs, or on the command line. Error messages and logs redact secret values.

### S3 and S3-compatible stores

| Variable | Purpose |
|---|---|
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | Access keys. Set both or neither. With neither, the default AWS chain is used (profile, SSO, instance role). |
| `AWS_SESSION_TOKEN` | Optional session token for temporary credentials. |
| `AWS_REGION` or `AWS_DEFAULT_REGION` | Region. Defaults to `us-east-1`. |
| `AWS_ENDPOINT_URL_S3` or `AWS_ENDPOINT_URL` | Custom endpoint, for example `https://minio.internal:9000`. Path-style addressing is used with a custom endpoint. |

### SFTP

| Variable | Purpose |
|---|---|
| `DAVILIZE_SFTP_USER` | User name, if the URL has none. The URL form `sftp://user@host/path` is preferred. |
| `DAVILIZE_SFTP_KEY` | Path to a private key file (OpenSSH format; ed25519 and RSA are supported). Tried first. |
| `DAVILIZE_SFTP_KEY_PASSPHRASE` | Passphrase for an encrypted key. |
| `DAVILIZE_SFTP_PASSWORD` | Password, used only if key auth is not configured or is refused. Never accepted from the URL. |
| `DAVILIZE_KNOWN_HOSTS` | Path to the known_hosts file. Default: `~/.ssh/known_hosts` (`%USERPROFILE%\.ssh\known_hosts` on Windows). |

Authentication needs `DAVILIZE_SFTP_KEY` or `DAVILIZE_SFTP_PASSWORD`. The ssh-agent is
not used, and default keys in `~/.ssh` are not tried. Set the key variable explicitly.

#### Host keys

The server's host key must match a line in known_hosts. Checks:

- **Known host, matching key:** connects.
- **Known host, different key:** always refused. This can mean the server was rebuilt
  or that the connection is being intercepted. Remove the old line only after you have
  confirmed the new key out of band.
- **Unknown host:** refused, and the error prints the key's SHA-256 fingerprint. Verify
  that fingerprint with the server's administrator, then either add the key to
  known_hosts yourself, or pass `--trust-new-host-key` once. That flag records the key
  on first use. It does not replace a key that is already recorded.

There is no option to accept any host key.

#### Upload behaviour

Each file is written to a hidden temporary name in the target folder, then renamed to
its final name. If the server refuses to rename over an existing file, the old file is
removed and the rename is retried once. Missing folders below the target root are
created. Remote paths are built from validated relative names, so `..` cannot leave the
target folder.

### Server refresh token

| Variable | Purpose |
|---|---|
| `DAVILIZE_REFRESH_TOKEN` | Bearer token for `POST /refresh`. If unset, the endpoint is disabled. |

## Manifest

Each publish contains `manifest.json`:

```json
{
  "format": "davilize-publish-manifest",
  "format_version": 1,
  "report": { "name": "Sample sales", "version": 1 },
  "generated_at": "2026-11-21T17:00:00Z",
  "files": [
    { "name": "index.html", "size": 12345, "sha256": "..." },
    { "name": "revenue_by_region.csv", "size": 120, "sha256": "..." }
  ]
}
```

The manifest lists every other file in the publish, with byte size and lowercase
hex SHA-256. It does not list itself. `generated_at` is UTC, RFC 3339.

## Atomicity and file names

- Each file is written to a hidden temporary name and renamed into place where the
  backend supports rename (local folders, SFTP). S3 has no rename, so it gets one
  direct write per object. S3 applies each PUT atomically.
- Files upload in this order: everything else, then `manifest.json`, then
  `index.html`. A reader that finds the index also finds the data it links to.
- A publish is not atomic as a whole. If it fails part way, earlier files stay in
  place. Run it again to complete the publish.
- Local and S3 publishes read each file into memory before uploading it. SFTP
  publishes do the same. Keep individual outputs to a size your machine can hold.
- Names must be relative. Only ASCII letters, digits, `-`, `_`, `.` and `/` are
  allowed. `..`, absolute paths, drive letters, backslashes, hidden (leading `.`)
  components, and Windows reserved names (`CON`, `NUL`, `COM1`, ...) are refused.
  There are at most 255 files per publish.

## Serving

`davilize serve <dir>` serves the files in `<dir>` over HTTP.

- Binds to `127.0.0.1` by default. Any other address needs `--bind`, and is logged
  with a warning. The server has no TLS. Put a reverse proxy in front of it for
  remote access.
- Paths are checked segment by segment. Encoded `..`, encoded slashes, backslashes,
  hidden names and symlinks that resolve outside the folder all get `404`. The
  request is not normalised first, so spellings such as `%2e%2e/` are refused too.
- Content types come from the file extension. Text types carry `charset=utf-8`.
- Every response has `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: no-referrer`, and `Cache-Control: no-cache`, plus a
  Content-Security-Policy:
  - HTML pages: scripts may come from the same origin, inline blocks, or
    `cdn.jsdelivr.net`. `'unsafe-eval'` is included because vega's expression code
    generator uses `Function`. Frames are not allowed.
  - Other files: no scripts, and no connections.

### Refreshing in the background

With `--refresh`, `<dir>` holds generations of output, each in a hidden `.gen-*`
folder. The report is re-run every `--every` seconds (default 300) and on
`POST /refresh`. A new generation is built completely before it replaces the served
one, so readers never see a mix of old and new files. A failed refresh keeps the
previous generation.

```
curl -X POST -H "Authorization: Bearer $DAVILIZE_REFRESH_TOKEN" http://127.0.0.1:8080/refresh
```

The endpoint answers `202` when the refresh is queued, `401` for a missing or wrong
token, and `404` when no token is set or the server was started without `--refresh`.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Invalid command line |
| 3 | Input problem: report or data missing or invalid, invalid target URL, invalid file name, served folder missing |
| 4 | Query problem: SQL error, or a visual that cannot be compiled |
| 5 | Output problem: cannot create or write local output files |
| 6 | Publish or network problem: missing or inconsistent credentials, host key refused, authentication failed, connection failed, upload refused or interrupted, or the server cannot listen |

## Testing

- `cargo test --workspace` runs the unit and integration tests. It needs no
  credentials and no network. It covers target parsing, name validation, manifest
  digests, the static server, the known_hosts decision logic, and local publishing.
  Live tests are skipped unless their environment variables are set.
- The SFTP upload path has not been run in `cargo test`. It was verified by hand
  against a local OpenSSH server (Git Bash `sshd`, key auth, port 2222): first contact
  refused, `--trust-new-host-key` recorded the key, a repeat publish succeeded,
  a changed key was refused even with the trust flag, and a wrong client key was
  refused with exit code 6.
- `DAVILIZE_LIVE_S3_TARGET=s3://bucket/prefix` (with the AWS variables) runs a real
  S3 round trip in `crates/dv-publish/tests/live.rs`.
- `DAVILIZE_LIVE_SFTP_TARGET=sftp://user@host/path` (with `DAVILIZE_SFTP_KEY` or
  `DAVILIZE_SFTP_PASSWORD`) runs a real SFTP round trip in the same file. Set
  `DAVILIZE_LIVE_SFTP_TRUST=1` to record an unknown host key on first use.

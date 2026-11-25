# Davilize

A local-first business intelligence tool in the spirit of Tableau and Power BI. A Rust engine does the work, a desktop app (Tauri + React) is the designer, and a command-line tool refreshes the same reports headlessly, so a report built in the app can run on a schedule with no GUI.

Status: early (version 0.1). It works end to end on the paths listed below, but several parts have not been verified against real services. See [Status](#status).

## What it does

- **Reads data** from CSV, Parquet, Arrow IPC and JSON files, and from SQLite and PostgreSQL databases. Queries run in an embedded DuckDB, which spills to disk, so data larger than memory is fine. Aggregation is pushed down to SQL.
- **Designs reports** in the app: pick fields, drop them on shelves, and get a bar, line, scatter, table or heatmap chart. Add filters (on rows or on aggregates), calculated fields and typed parameters.
- **Joins datasets** on matching columns, as inner, left, right or full joins, in the app or in the report file. A joined dataset can join others, and fields and charts use it like any other dataset.
- **Lays out dashboards** on a grid, with drag and keyboard editing and filters shared across visuals.
- **Exports** to HTML (interactive, optionally fully offline), SVG, PNG, PDF, CSV, Parquet and XLSX.
- **Publishes** a refreshed report to a folder, S3 (or S3-compatible storage) or SFTP, and can serve a folder over HTTP.
- **Refreshes from the CLI** with parameter overrides and stable exit codes, for cron or Task Scheduler.
- **Schedules** refreshes and publishes: a cron-style schedule in the report, run by `davilize schedule run` as a daemon or once with `--now` from Task Scheduler or cron.

A report is one deterministic JSON file (`.dvz`) that diffs cleanly and never holds credentials. See [docs/reports.md](docs/reports.md).

## Quick start

You need a recent Rust toolchain. The app also needs Node.js. On Linux, Tauri needs the WebKitGTK and GTK development packages; see `.github/workflows/ci.yml` for the list.

Build the CLI and try the examples:

```
cargo build --release -p dv-cli
target/release/davilize inspect examples/sample.csv
target/release/davilize refresh examples/sample.dvz --out out --format html,csv
target/release/davilize refresh examples/sample_dashboard.dvz --out out --format html --param region=South
```

The first build compiles DuckDB and takes several minutes.

Run the desktop app:

```
cd app
npm install
npm run tauri dev
```

## Command line

```
davilize inspect <file>                    Schema and column profile of a data file
davilize run-sql <file|report> "<sql>"     Run SQL over a data file or a report's datasets
davilize refresh <report> --out DIR --format html,svg,png,pdf,csv,parquet,xlsx
davilize publish <report> --target <folder|sftp://...|s3://...>
davilize serve <dir> [--refresh <report>]
davilize param list <report> [--json]      Show declared parameters
davilize schedule list <report>            Show schedules and their next run times
davilize schedule run <report> [--now]     Run schedules as a daemon, or once with --now
```

Exit codes: `0` ok, `2` usage, `3` input, `4` query or spec, `5` output, `6` publish or network.

## Documentation

- [Report files, datasets, filters, parameters and dashboards](docs/reports.md)
- [Publishing targets, credentials and the built-in server](docs/publishing.md)
- [Tableau and Power BI import and export](docs/interop.md)
- [Running scheduled reports](docs/scheduling.md)

## Layout

| Path | Contents |
|---|---|
| `crates/dv-core` | Report model, validation, errors |
| `crates/dv-engine` | DuckDB engine, connectors, spec-to-SQL compiler, calculated-field checks |
| `crates/dv-render` | Chart specs and exports (HTML, SVG, PNG, PDF, CSV, Parquet, XLSX) |
| `crates/dv-publish` | Folder, S3 and SFTP targets, static server |
| `crates/dv-cli` | The `davilize` binary |
| `app/` | Tauri shell (`src-tauri`) and React frontend (`src`) |
| `examples/` | Sample data and reports |

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cd app && npx tsc --noEmit && npm run test:layout && npm run build
```

## Security notes

- Reports contain no secrets. Database sources hold a connection name, resolved from `DAVILIZE_CONN_<NAME>`. Publishing credentials come from environment variables only.
- Calculated-field expressions are parsed by DuckDB and checked against an allowlist before any query runs.
- SFTP uses a pure-Rust client and checks host keys against `known_hosts`.

## Status

Verified: the Rust test suite, the CLI, HTML export and the desktop app on Windows, including the dashboard editor.

Not yet verified: publishing to a real S3 service or a live SFTP server, the app on macOS and Linux, PostgreSQL against a live server, and the CI workflow (it has not run). There are no installers yet.

Not built yet: multi-user features, a Python bridge, and more connectors (MySQL, Snowflake, BigQuery, Azure Blob, GCS).

## Web version

The `web/` folder is a static page that runs reports in the browser. The `dv-wasm` crate compiles `.dvz` reports to SQL with the same Rust code as the CLI, and [DuckDB-Wasm](https://github.com/duckdb/duckdb-wasm) runs the queries. It handles file sources, custom SQL, calculated fields and parameters (default values), but not joins or database sources.

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli   # same version as wasm-bindgen in Cargo.lock
bash scripts/build-site.sh       # writes ./site
```

The `Web site` workflow builds the same folder and deploys it to GitHub Pages on every push to `main`. In the repository settings, set Pages to deploy from GitHub Actions.

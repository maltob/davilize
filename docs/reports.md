# Report files (`.dvz`)

A report is one JSON file, version 1. Output is deterministic (pretty-printed, fixed key order), so reports diff cleanly. Unknown versions are rejected. Sections default to empty when omitted, and newer optional fields are omitted when unused, so older files round-trip unchanged.

Reports never contain secrets. A database source holds only a connection reference name, which the CLI resolves from the environment variable `DAVILIZE_CONN_<NAME>`.

```json
{
  "version": 1,
  "name": "Sample sales",
  "sources":    [ ... ],
  "datasets":   [ ... ],
  "visuals":    [ ... ],
  "parameters": [ ... ],
  "dashboards": [ ... ]
}
```

Names (sources, datasets, visual ids, parameters, calculated fields) use only letters, digits, `_` and `-`. Sources and datasets share one namespace.

## Sources

```json
{ "name": "sales_csv", "type": "file", "path": "sample.csv", "format": "csv" }
{ "name": "warehouse", "type": "database", "kind": "postgres", "connection": "warehouse-prod" }
```

- File formats: `csv`, `parquet`, `arrow`, `json`. Relative paths resolve against the report's directory.
- Database kinds: `sqlite`, `postgres` (attached read-only).

## Datasets

A dataset is a named relation over one source. Without `sql` it exposes the whole source. With `sql` it is the result of that query.

```json
{ "name": "sales", "source": "sales_csv", "sql": "SELECT * FROM sales_csv WHERE units > 0",
  "calculated_fields": [ ... ] }
```

### Calculated fields

```json
{ "name": "net_revenue", "expression": "revenue * 0.9", "description": "optional" }
```

Each calculated field adds a column computed per row. A field may use the fields defined before it in the list. The expression is parsed by DuckDB, and the syntax tree is checked against an allowlist before any query runs. Anything else is refused with an error that names the field.

Allowed:
- Column references, constants, and `CASE` expressions.
- `CAST` to a built-in scalar type: BOOLEAN, TINYINT, SMALLINT, INTEGER, BIGINT, HUGEINT, the unsigned integer types, FLOAT, REAL, DOUBLE, DECIMAL, NUMERIC, VARCHAR, TEXT, STRING, CHAR, DATE, TIME, TIMESTAMP, INTERVAL.
- Operators: `+ - * / // % ^ ** ||`, comparisons (`= <> != < <= > >=`), `LIKE` and `ILIKE` forms, plus `AND`, `OR`, `NOT`, `IS [NOT] NULL`, `IN` and `BETWEEN`.
- Functions:
  - Math: abs, round, round_even, floor, ceil, ceiling, trunc, sqrt, cbrt, power, pow, exp, ln, log, log10, log2, mod, sign, pi, degrees, radians, sin, cos, tan, asin, acos, atan, atan2, greatest, least.
  - Text: length, char_length, lower, upper, trim, ltrim, rtrim, substring, substr, replace, concat, concat_ws, left, right, reverse, starts_with, ends_with, contains.
  - Conditional: coalesce, ifnull, nullif, if, iif.
  - Dates: year, month, day, hour, minute, second, dayofweek, dayofyear, weekday, week, quarter, date_part, date_trunc, datediff, date_diff, date_add, date_sub, make_date, strftime, strptime.

Refused: subqueries, window functions, `*`, aliases, bind parameters, aggregates and any function not listed (including table functions, `read_*`, `getenv`, `current_setting`, clock and random functions, and `repeat`, `lpad`, `rpad`, which can allocate unbounded memory). Also refused: comments (`--`, `/* */`), `;`, control characters, and expressions over 2000 characters.

The allowlist lives in `crates/dv-engine/src/calc.rs`. Change this document and that file together.

## Visuals

```json
{
  "id": "revenue_by_region", "title": "Revenue by region", "dataset": "sales",
  "chart": "bar",
  "encodings": { "x": "region", "y": "total_revenue", "color": null, "size": null },
  "dimensions": ["region"],
  "measures": [ { "field": "revenue", "agg": "sum", "alias": "total_revenue" } ],
  "filters": [ ... ],
  "sort": { "by": "total_revenue", "direction": "desc" },
  "limit": 10
}
```

- `chart`: `bar`, `line`, `scatter`, `table`, `heatmap`.
- `agg`: `sum`, `avg`, `min`, `max`, `count`, `count_distinct`. Without an alias, the output column is `<agg>_<field>`.
- `sort.direction`: `asc` (default) or `desc`. Grouped results are tie-broken by the dimensions, so row order is stable.
- `limit`: a number, or `{"param": "name"}`.

### Filters

```json
{ "field": "region", "op": "in", "values": ["North", "South"] }
{ "field": "total_revenue", "op": "gt", "values": [1000], "scope": "aggregate" }
```

- `op`: `eq`, `ne`, `lt`, `le`, `gt`, `ge`, `in`, `between` (inclusive, exactly two values: lower then upper), `is_null`, `is_not_null` (no values).
- `scope`: `row` (default, a `WHERE` on a dataset column, applied before grouping) or `aggregate` (a `HAVING` on a measure's output name, applied after grouping).
- A value is a literal (string, number, bool) or a parameter reference `{"param": "name"}`.

## Parameters

```json
{ "name": "top", "type": "integer", "default": 5, "min": 1, "max": 50, "description": "..." }
{ "name": "region", "type": "string", "allowed": ["North", "South"] }
```

- `type`: `string`, `number`, `integer`, `boolean`, `date` (`YYYY-MM-DD`). When absent, the type is taken from `default`, or `string` if there is none.
- A parameter with no `default` is required.
- `allowed` restricts the accepted values. `min` and `max` are inclusive bounds for number, integer and date parameters.
- Every `{"param": ...}` reference must name a declared parameter, or the report is invalid.

Values come from `--param NAME=VALUE` (repeatable) or the default. Each value is parsed and checked against the type and constraints. A bad value stops the run with exit code 3 and writes nothing.

## Dashboards

A dashboard places visuals on a grid and shares filters among them. A report without dashboards is unchanged.

```json
{
  "id": "overview", "title": "Overview", "columns": 12, "row_height": 80,
  "items": [
    { "visual": "revenue_by_region", "x": 0, "y": 0, "w": 6, "h": 4 },
    { "visual": "product_revenue", "x": 6, "y": 0, "w": 6, "h": 4 }
  ],
  "filters": [ { "field": "region", "op": "eq", "values": [ { "param": "region" } ] } ]
}
```

- `id`: unique among dashboards, same name rules as visual ids. `title` is shown above the grid.
- `columns`: 1 to 24, default 12. `row_height`: the height of one grid row in pixels, 20 to 400, default 80.
- `items`: each has `visual` (the id of a visual in the report), and `x`, `y` (0-based grid position, default 0), `w` and `h` (size in grid units, at least 1). Each visual appears at most once per dashboard. An item must fit inside `columns` (`x + w <= columns`), and items must not overlap.
- `filters`: shared filters, written like visual filters but row scope only (see below). Optional; omitted when empty.

A report's `dashboards` array is omitted when empty, so reports written before dashboards existed serialise unchanged. Validation failures (unknown or duplicate ids, a visual placed twice or outside the grid, overlapping items, grid sizes out of range, an aggregate-scope or malformed shared filter, an undeclared parameter) make the report invalid, which is exit code 3.

### Shared filter semantics

A dashboard filter applies to every visual on that dashboard whose dataset has a column named `field` (matched exactly). A visual whose dataset lacks that column is shown without the filter. A shared filter is added after the visual's own filters, and `{"param": ...}` values are bound with the report's parameters. Parameters stay global: one `--param region=North` applies to every dashboard.

A visual placed on two dashboards is rendered once per placement, each with that dashboard's filters, so the two charts can show different numbers.

### Layout and other formats

- `refresh --format html` (and `publish` and `serve --refresh`, which write HTML) draws one section per dashboard. Each section has a heading and a CSS grid, with `grid-template-columns: repeat(columns, 1fr)` and `grid-auto-rows: <row_height>px`. Each chart sits in its cell, with its title, and resizes with the cell. At 700px wide or less, the grid becomes one column with the cells stacked top to bottom, then left to right.
- When a report has dashboards, the HTML page shows only the dashboards. Visuals that are on no dashboard are not shown.
- The other formats do not use dashboards. `csv`, `parquet`, `svg`, `png`, `pdf` and `xlsx` are written per visual, without dashboard filters or grid, exactly as for a report with no dashboards.

`examples/sample_dashboard.dvz` has two dashboards, a shared `region` parameter filter, and one visual on both dashboards:

```
davilize refresh examples/sample_dashboard.dvz --out out --format html --param region=South
```

## CLI

```
davilize inspect <file>                    Schema and column profile of a data file
davilize run-sql <file|report> "<sql>"     Run SQL over a data file or a report's datasets
davilize refresh <report> --out DIR --format html,svg,png,pdf,csv,parquet,xlsx
davilize publish <report> --target <folder|sftp://...|s3://...>
davilize serve <dir>  [--refresh <report>]
davilize param list <report> [--json]      Show declared parameters
```

`refresh`, `publish`, `run-sql` and `serve --refresh` accept `--param NAME=VALUE`. In `run-sql`, each parameter is also available as a macro `param_<name>()`. See [publishing.md](publishing.md) for targets and the server.

Exit codes: `0` ok, `2` usage, `3` input (including bad parameters), `4` query or spec, `5` output, `6` publish or network.

`examples/sample_params.dvz` shows filters on measures, calculated fields and parameters together:

```
davilize refresh examples/sample_params.dvz --out out --format html --param top=3 --param region=South
```

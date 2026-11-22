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
  "parameters": [ ... ]
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

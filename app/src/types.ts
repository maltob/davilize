// Types that mirror the JSON of dv-core (report model) and the app's Tauri commands.

export type Aggregate = "sum" | "avg" | "min" | "max" | "count" | "count_distinct";
export type ChartType = "bar" | "line" | "scatter" | "table" | "heatmap";
/** Filter operators. `between` takes exactly two values: lower, then upper (inclusive). */
export type FilterOp =
  | "eq"
  | "ne"
  | "lt"
  | "le"
  | "gt"
  | "ge"
  | "in"
  | "between"
  | "is_null"
  | "is_not_null";
/** `row` filters dataset rows before grouping (WHERE); `aggregate` filters measure totals (HAVING). */
export type FilterScope = "row" | "aggregate";
export type ParamType = "string" | "number" | "integer" | "boolean" | "date";
export type Family = "number" | "text" | "date" | "bool" | "other";

/** A `{"param": "<name>"}` reference, written in place of a literal value or limit. */
export interface ParamRef {
  param: string;
}

/** A visual's row limit: a number, or a parameter reference. */
export type Limit = number | ParamRef;

export interface Encodings {
  x?: string | null;
  y?: string | null;
  color?: string | null;
  size?: string | null;
}

export interface Measure {
  field: string;
  agg: Aggregate;
  alias?: string | null;
}

export interface Filter {
  field: string;
  op: FilterOp;
  /** Literals or {"param": name} references. Empty for is_null and is_not_null. */
  values: unknown[];
  /** Omitted for row filters, so existing files serialise unchanged. */
  scope?: FilterScope;
}

export interface Sort {
  by: string;
  direction: "asc" | "desc";
}

export interface Visual {
  id: string;
  title: string;
  dataset: string;
  chart: ChartType;
  encodings: Encodings;
  dimensions: string[];
  measures: Measure[];
  filters: Filter[];
  sort?: Sort | null;
  limit?: Limit | null;
}

export type Source =
  | { name: string; type: "file"; path: string; format: "csv" | "parquet" | "arrow" | "json" }
  | { name: string; type: "database"; kind: "sqlite" | "postgres"; connection: string };

/** A column computed per row from an expression over earlier columns (see docs/reports.md). */
export interface CalculatedField {
  name: string;
  expression: string;
  description?: string | null;
}

export interface Dataset {
  name: string;
  source: string;
  sql?: string | null;
  /** Omitted when empty. */
  calculated_fields?: CalculatedField[];
}

/** A report parameter. `type` is omitted when it is implied by `default`. */
export interface Parameter {
  name: string;
  type?: ParamType;
  default?: unknown;
  description?: string | null;
  allowed?: unknown[];
  min?: unknown;
  max?: unknown;
}

export interface Report {
  version: number;
  name: string;
  sources: Source[];
  datasets: Dataset[];
  visuals: Visual[];
  parameters: Parameter[];
}

export interface ColumnView {
  name: string;
  data_type: string;
  family: Family;
  role: "measure" | "dimension";
  /** True for columns added by a calculated field. */
  calculated?: boolean;
}

export interface DatasetSchema {
  name: string;
  columns: ColumnView[];
}

export interface AddedDataset {
  source: Source;
  dataset: Dataset;
  columns: ColumnView[];
}

export interface OpenedReport {
  report: Report;
  path: string;
  datasets: DatasetSchema[];
}

export interface QueryTable {
  columns: { name: string; data_type: string }[];
  rows: (string | null)[][];
}

export interface RenderedVisual {
  id: string;
  title: string;
  spec: Record<string, unknown>;
  notes: string[];
  sql: string;
  row_count: number;
}

export interface ExportSummary {
  files: string[];
  warnings: string[];
}

export interface ApiError {
  kind: "cancelled" | "invalid" | "query" | "source" | "io" | "output" | "internal" | string;
  message: string;
}

// Types that mirror the JSON of dv-core (report model) and the app's Tauri commands.

export type Aggregate = "sum" | "avg" | "min" | "max" | "count" | "count_distinct";
export type ChartType = "bar" | "line" | "scatter" | "table" | "heatmap";
export type FilterOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "in" | "is_null" | "is_not_null";
export type Family = "number" | "text" | "date" | "bool" | "other";

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
  values: unknown[];
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
  limit?: number | null;
}

export type Source =
  | { name: string; type: "file"; path: string; format: "csv" | "parquet" | "arrow" | "json" }
  | { name: string; type: "database"; kind: "sqlite" | "postgres"; connection: string };

export interface Dataset {
  name: string;
  source: string;
  sql?: string | null;
}

export interface Report {
  version: number;
  name: string;
  sources: Source[];
  datasets: Dataset[];
  visuals: Visual[];
  parameters: unknown[];
}

export interface ColumnView {
  name: string;
  data_type: string;
  family: Family;
  role: "measure" | "dimension";
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

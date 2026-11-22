// Pure helpers for a chart drawn from a dv-render Vega-Lite spec: a text summary (the chart's
// accessible name), the values as a table, and the config that lets a chart use the app's
// colours. No React or DOM here, so the node tests can run them. The summary wording follows
// dv-render's summary.rs, so the app and the HTML report describe a chart the same way.

export type ChartSpec = Record<string, unknown>;

/** Longest summary, in characters. Longer text is cut and ends with "...". */
export const SUMMARY_MAX_CHARS = 250;
/** Rows in the table view. The rest are left out, and the table says so. */
export const TABLE_MAX_ROWS = 200;

export type CellValue = string | number | null;

export interface ChartTable {
  columns: string[];
  /** The first TABLE_MAX_ROWS rows, in the order of the spec. */
  rows: CellValue[][];
  totalRows: number;
  truncated: boolean;
  /** True for columns whose values are all numbers (or blank), which the table right-aligns. */
  numeric: boolean[];
}

export interface SpecReading {
  kind: "bar" | "line" | "scatter" | "heatmap" | "table" | "other";
  rows: Record<string, unknown>[];
  x?: string;
  y?: string;
  color?: string;
  size?: string;
  columns: string[];
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function fieldOf(channel: unknown): string | undefined {
  if (!isRecord(channel) || typeof channel.field !== "string") return undefined;
  return channel.field;
}

/** What kind of chart a spec draws, its channels, and its data rows (in spec order). */
export function readSpec(spec: ChartSpec): SpecReading {
  const data = isRecord(spec.data) && Array.isArray(spec.data.values) ? spec.data.values.filter(isRecord) : [];

  // dv-render draws a table as one text column per result column, with a row index to keep order.
  if (isRecord(spec.repeat) && Array.isArray(spec.repeat.column)) {
    const columns = spec.repeat.column.filter((c): c is string => typeof c === "string");
    const rows = [...data].sort((a, b) => Number(a.__row ?? 0) - Number(b.__row ?? 0));
    return { kind: "table", rows, columns };
  }

  const encoding: Record<string, unknown> = isRecord(spec.encoding) ? spec.encoding : {};
  const mark = typeof spec.mark === "string" ? spec.mark : isRecord(spec.mark) ? spec.mark.type : undefined;
  const kind =
    mark === "bar" ? "bar" : mark === "line" ? "line" : mark === "point" ? "scatter" : mark === "rect" ? "heatmap" : "other";
  const x = fieldOf(encoding.x);
  const y = fieldOf(encoding.y);
  const color = fieldOf(encoding.color);
  const size = fieldOf(encoding.size);
  const columns = [...new Set([x, y, color, size].filter((c): c is string => Boolean(c)))];
  return { kind, rows: data, x, y, color, size, columns };
}

/** A value as it appears in a table or a summary. Non-finite numbers are blank. */
export function toCell(value: unknown): CellValue {
  if (value === null || value === undefined) return null;
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "string") return value;
  if (typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

/** Up to two decimals, trailing zeros dropped. Matches dv-render's fmt_number. */
export function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return "n/a";
  if (Number.isInteger(value) && Math.abs(value) < 1e15) return String(value);
  return value.toFixed(2).replace(/\.?0+$/, "");
}

function labelOf(value: unknown): string {
  const cell = toCell(value);
  if (cell === null) return "(blank)";
  return typeof cell === "number" ? formatNumber(cell) : String(cell);
}

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

interface Extreme {
  label: string;
  value: number;
}

/** The highest and lowest finite values of `field`, each labelled. Undefined when there are none. */
function extremes(
  rows: Record<string, unknown>[],
  field: string | undefined,
  label: (row: Record<string, unknown>) => string,
): { high: Extreme; low: Extreme } | undefined {
  if (!field) return undefined;
  let high: Extreme | undefined;
  let low: Extreme | undefined;
  for (const row of rows) {
    const value = row[field];
    if (typeof value !== "number" || !Number.isFinite(value)) continue;
    const item = { label: label(row), value };
    if (!high || value > high.value) high = item;
    if (!low || value < low.value) low = item;
  }
  return high && low ? { high, low } : undefined;
}

function rangeOf(rows: Record<string, unknown>[], field: string | undefined): [number, number] | undefined {
  if (!field) return undefined;
  const values = rows.map((row) => row[field]).filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  if (values.length === 0) return undefined;
  return [Math.min(...values), Math.max(...values)];
}

function extremesText(found: { high: Extreme; low: Extreme } | undefined): string {
  if (!found) return "";
  return ` Highest: ${found.high.label} ${formatNumber(found.high.value)}, lowest: ${found.low.label} ${formatNumber(found.low.value)}.`;
}

function summaryText(reading: SpecReading): string {
  const { rows } = reading;
  const n = rows.length;
  switch (reading.kind) {
    case "bar":
    case "line": {
      const kind = reading.kind === "bar" ? "Bar chart" : "Line chart";
      const noun = reading.kind === "bar" ? count(n, "bar", "bars") : count(n, "point", "points");
      const found = extremes(rows, reading.y, (row) => {
        const label = labelOf(row[reading.x ?? ""]);
        return reading.color ? `${label} (${labelOf(row[reading.color])})` : label;
      });
      return `${kind}: ${reading.y ?? "value"} by ${reading.x ?? "category"}. ${noun}.${extremesText(found)}`;
    }
    case "scatter": {
      const x = reading.x ?? "x";
      const y = reading.y ?? "y";
      let text = `Scatter plot: ${y} against ${x}. ${count(n, "point", "points")}.`;
      const xr = rangeOf(rows, reading.x);
      const yr = rangeOf(rows, reading.y);
      if (xr && yr) {
        text += ` ${x} from ${formatNumber(xr[0])} to ${formatNumber(xr[1])}, ${y} from ${formatNumber(yr[0])} to ${formatNumber(yr[1])}.`;
      }
      return text;
    }
    case "heatmap": {
      const found = extremes(
        rows,
        reading.color,
        (row) => `${labelOf(row[reading.x ?? ""])}, ${labelOf(row[reading.y ?? ""])}`,
      );
      return `Heatmap: ${reading.color ?? "value"} by ${reading.x ?? "x"} and ${reading.y ?? "y"}. ${count(n, "cell", "cells")}.${extremesText(found)}`;
    }
    case "table": {
      const names = reading.columns.length > 0 ? ` Columns: ${reading.columns.join(", ")}.` : "";
      return `Table: ${count(n, "row", "rows")}.${names}`;
    }
    default:
      return `Chart: ${count(n, "value", "values")}.`;
  }
}

/** Cuts text to SUMMARY_MAX_CHARS characters, ending with "..." when it is cut. */
export function capText(text: string): string {
  const chars = [...text];
  if (chars.length <= SUMMARY_MAX_CHARS) return text;
  return `${chars.slice(0, SUMMARY_MAX_CHARS - 3).join("").trimEnd()}...`;
}

/** A one-line text description of a chart, used as its accessible name. Capped at SUMMARY_MAX_CHARS. */
export function chartSummary(spec: ChartSpec): string {
  return capText(summaryText(readSpec(spec)));
}

/** The chart's values as a table, in the spec's order, with rows capped at TABLE_MAX_ROWS. */
export function chartTable(spec: ChartSpec): ChartTable {
  const reading = readSpec(spec);
  const all = reading.rows.map((row) => reading.columns.map((column) => toCell(row[column])));
  const numeric = reading.columns.map((_, index) => {
    const values = all.map((row) => row[index]).filter((v) => v !== null);
    return values.length > 0 && values.every((v) => typeof v === "number");
  });
  return {
    columns: reading.columns,
    rows: all.slice(0, TABLE_MAX_ROWS),
    totalRows: all.length,
    truncated: all.length > TABLE_MAX_ROWS,
    numeric,
  };
}

/** Theme colours read from the app's CSS variables. */
export interface ChartColours {
  text: string;
  muted: string;
  line: string;
  grid: string;
}

/**
 * The Vega-Lite config that paints a chart in the app's colours, so it reads in light and
 * dark mode. Only text, axis and grid colours are set; the palette and sizes come from the spec.
 */
export function chartThemeConfig(colours: ChartColours): ChartSpec {
  return {
    axis: {
      labelColor: colours.text,
      titleColor: colours.text,
      domainColor: colours.line,
      tickColor: colours.line,
      gridColor: colours.grid,
    },
    legend: { labelColor: colours.text, titleColor: colours.text },
    title: { color: colours.text, subtitleColor: colours.muted },
  };
}

/** A deep merge that does not change its inputs. Values in `extra` win. */
export function mergeConfig(base: ChartSpec, extra: ChartSpec): ChartSpec {
  const out: ChartSpec = { ...base };
  for (const [key, value] of Object.entries(extra)) {
    const current = out[key];
    out[key] = isRecord(current) && isRecord(value) ? mergeConfig(current, value) : value;
  }
  return out;
}

/**
 * The spec as the app draws it: the Vega title is left out (the card shows the title), and the
 * theme is merged under the spec's own config. The input is not changed.
 */
export function themedSpec(spec: ChartSpec, theme: ChartSpec): ChartSpec {
  const out: ChartSpec = { ...spec };
  delete out.title;
  out.config = mergeConfig(theme, isRecord(spec.config) ? spec.config : {});
  return out;
}

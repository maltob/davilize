// Pure logic that maps the shelf-based chart builder onto a dv-core Visual, and back.
// The visual is the only state: shelves are derived from it, so a saved visual reopens exactly.
import type {
  Aggregate,
  CalculatedField,
  ColumnView,
  Dashboard,
  Dataset,
  Family,
  Filter,
  FilterOp,
  FilterScope,
  Join,
  JoinKey,
  JoinKind,
  Limit,
  Measure,
  ParamRef,
  ParamType,
  Parameter,
  Report,
  Visual,
} from "./types";
import { GRID_LIMITS } from "./layout.ts";

export type Slot = "x" | "y" | "color" | "size";
export type PillAgg = Aggregate | "raw";
export interface Pill {
  field: string;
  kind: "dimension" | "measure";
  agg: PillAgg;
}
export type Slots = Record<Slot, Pill | null>;

export const SLOTS: { key: Slot; label: string; hint: string }[] = [
  { key: "x", label: "Columns", hint: "x axis" },
  { key: "y", label: "Rows", hint: "y axis" },
  { key: "color", label: "Color", hint: "colour encoding" },
  { key: "size", label: "Size", hint: "size encoding (scatter)" },
];

export const AGGREGATES: { value: PillAgg; label: string }[] = [
  { value: "sum", label: "Sum" },
  { value: "avg", label: "Average" },
  { value: "min", label: "Minimum" },
  { value: "max", label: "Maximum" },
  { value: "count", label: "Count" },
  { value: "count_distinct", label: "Count distinct" },
  { value: "raw", label: "None (row values)" },
];

export const CHART_TYPES: { value: Visual["chart"]; label: string }[] = [
  { value: "bar", label: "Bar" },
  { value: "line", label: "Line" },
  { value: "scatter", label: "Scatter" },
  { value: "table", label: "Table" },
  { value: "heatmap", label: "Heatmap" },
];

/** How many values an operator takes: none, one, exactly two (lower, upper), or a list. */
export type ValueSlots = "none" | "one" | "two" | "list";

export const FILTER_OPS: { value: FilterOp; label: string; slots: ValueSlots }[] = [
  { value: "eq", label: "equals", slots: "one" },
  { value: "ne", label: "does not equal", slots: "one" },
  { value: "lt", label: "less than", slots: "one" },
  { value: "le", label: "at most", slots: "one" },
  { value: "gt", label: "greater than", slots: "one" },
  { value: "ge", label: "at least", slots: "one" },
  { value: "in", label: "is one of", slots: "list" },
  { value: "between", label: "is between (inclusive)", slots: "two" },
  { value: "is_null", label: "is empty", slots: "none" },
  { value: "is_not_null", label: "is not empty", slots: "none" },
];

export const FILTER_SCOPES: { value: FilterScope; label: string }[] = [
  { value: "row", label: "Rows" },
  { value: "aggregate", label: "Totals (after grouping)" },
];

export const PARAM_TYPES: { value: ParamType; label: string }[] = [
  { value: "string", label: "Text" },
  { value: "number", label: "Number" },
  { value: "integer", label: "Whole number" },
  { value: "boolean", label: "Yes or no" },
  { value: "date", label: "Date" },
];

export function outputName(measure: Measure): string {
  return measure.alias ?? `${measure.agg}_${measure.field}`;
}

export function emptySlots(): Slots {
  return { x: null, y: null, color: null, size: null };
}

export function blankVisual(dataset: string, id: string): Visual {
  return {
    id,
    title: "",
    dataset,
    chart: "bar",
    encodings: {},
    dimensions: [],
    measures: [],
    filters: [],
    sort: null,
    limit: null,
  };
}

/** A visual id that is not used in the report. Ids become file names, so only [A-Za-z0-9_-]. */
export function newVisualId(report: Report): string {
  const used = new Set(report.visuals.map((v) => v.id));
  let n = report.visuals.length + 1;
  while (used.has(`visual_${n}`)) n += 1;
  return `visual_${n}`;
}

export function slotsFromVisual(visual: Visual): Slots {
  const slots = emptySlots();
  for (const { key } of SLOTS) {
    const name = visual.encodings[key];
    if (!name) continue;
    const measure = visual.measures.find((m) => outputName(m) === name);
    if (measure) {
      slots[key] = { field: measure.field, kind: "measure", agg: measure.agg };
    } else if (visual.dimensions.includes(name)) {
      slots[key] = { field: name, kind: "dimension", agg: "sum" };
    } else {
      // Not grouped: a raw row value (scatter points, or table columns).
      slots[key] = { field: name, kind: "measure", agg: "raw" };
    }
  }
  return slots;
}

/** Writes the shelves into a visual's encodings, dimensions and measures. Other fields are kept. */
export function applySlots(visual: Visual, slots: Slots): Visual {
  const encodings: Visual["encodings"] = {};
  const dimensions: string[] = [];
  const measures: Measure[] = [];
  for (const { key } of SLOTS) {
    const pill = slots[key];
    if (!pill) {
      encodings[key] = null;
    } else if (pill.kind === "dimension") {
      if (!dimensions.includes(pill.field)) dimensions.push(pill.field);
      encodings[key] = pill.field;
    } else if (pill.agg === "raw") {
      encodings[key] = pill.field;
    } else {
      const measure: Measure = { field: pill.field, agg: pill.agg };
      const name = outputName(measure);
      if (!measures.some((m) => outputName(m) === name)) measures.push(measure);
      encodings[key] = name;
    }
  }
  return { ...visual, encodings, dimensions, measures };
}

export function hasFields(visual: Visual): boolean {
  // Hand-written reports (e.g. table visuals) may define columns only through
  // dimensions/measures, with no encodings.
  return (
    Object.values(visual.encodings).some((v) => Boolean(v)) ||
    visual.dimensions.length > 0 ||
    visual.measures.length > 0
  );
}

/** Field names a sort or a shelf can refer to. */
export function outputNames(visual: Visual): string[] {
  const names = [
    ...visual.dimensions,
    ...visual.measures.map(outputName),
    ...Object.values(visual.encodings).filter((v): v is string => Boolean(v)),
  ];
  return [...new Set(names)];
}

/**
 * The aggregates that make sense for a field of this type. Summing or averaging text, dates or
 * true/false values is an error in SQL, so those fields are offered counts (and min and max
 * where they order: text and dates). The first entry is the default.
 */
export function aggregatesFor(family: Family | undefined): PillAgg[] {
  switch (family) {
    case "number":
      return ["sum", "avg", "min", "max", "count", "count_distinct", "raw"];
    case "text":
      return ["count_distinct", "count", "min", "max", "raw"];
    case "date":
      return ["count", "count_distinct", "min", "max", "raw"];
    default:
      return ["count", "count_distinct", "raw"];
  }
}

/**
 * How a field is aggregated when it lands on a measure shelf: sum for a real quantity, count
 * distinct for text and identifiers (an order_id summed means nothing), count for the rest.
 */
export function defaultAgg(column: ColumnView): PillAgg {
  if (column.family === "number") return column.role === "measure" ? "sum" : "count_distinct";
  if (column.family === "text") return "count_distinct";
  return "count";
}

/** What a shelf holds for a chart type: a grouping field, a number to aggregate, or either. */
export function expectedKind(chart: Visual["chart"], slot: Slot): Pill["kind"] | null {
  switch (chart) {
    case "bar":
    case "line":
      return slot === "x" || slot === "color" ? "dimension" : "measure";
    case "scatter":
      return slot === "color" ? "dimension" : "measure";
    case "heatmap":
      return slot === "color" ? "measure" : slot === "size" ? "measure" : "dimension";
    case "table":
      return null;
  }
}

/**
 * Default pill for a field dropped on a shelf. The shelf decides the kind (a text field on a
 * measure shelf becomes a count, a number on a grouping shelf groups by its values); with no
 * shelf in mind, numbers are measures and everything else groups.
 */
export function pillFor(column: ColumnView, slot?: Slot, chart?: Visual["chart"]): Pill {
  const wanted = slot && chart ? expectedKind(chart, slot) : null;
  const kind = wanted ?? (column.role === "measure" ? "measure" : "dimension");
  return kind === "measure"
    ? { field: column.name, kind, agg: defaultAgg(column) }
    : { field: column.name, kind, agg: "sum" };
}

/**
 * The shelf a field should go to when it is dropped on the chart itself, not on a shelf: the
 * first empty shelf that takes that kind of field (numbers to measure shelves, everything else to
 * grouping shelves), then any empty shelf, then the main shelf of that kind, which it replaces.
 */
export function smartSlot(chart: Visual["chart"], column: ColumnView, slots: Slots): Slot {
  const wantsMeasure = column.family === "number" && column.role === "measure";
  const want: Pill["kind"] = wantsMeasure ? "measure" : "dimension";
  const order: Slot[] = wantsMeasure ? ["y", "x", "size", "color"] : ["x", "y", "color", "size"];
  const usable = order.filter((slot) => slot !== "size" || chart === "scatter");
  const kindOf = (slot: Slot) => expectedKind(chart, slot) ?? want;
  return (
    usable.find((slot) => !slots[slot] && kindOf(slot) === want) ??
    usable.find((slot) => !slots[slot]) ??
    usable.find((slot) => kindOf(slot) === want) ??
    "x"
  );
}

export function autoTitle(visual: Visual): string {
  const names = outputNames(visual);
  return names.length ? names.join(" by ") : "Untitled visual";
}

export function familyOf(columns: ColumnView[] | undefined, name: string): Family | undefined {
  return columns?.find((c) => c.name === name)?.family;
}

// ---------------------------------------------------------------------------
// Filters: scope, field choices, and values that are literals or parameters
// ---------------------------------------------------------------------------

/** Same test as dv-core: exactly one key, `param`, holding a string. */
export function isParamRef(value: unknown): value is ParamRef {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return keys.length === 1 && keys[0] === "param" && typeof (value as { param: unknown }).param === "string";
}

/** A filter's scope. Filters without one are row filters. */
export function filterScope(filter: Filter): FilterScope {
  return filter.scope === "aggregate" ? "aggregate" : "row";
}

/** Sets a filter's scope. Row is the default, so the key is removed for it and files stay unchanged. */
export function withScope(filter: Filter, scope: FilterScope): Filter {
  const next: Filter = { ...filter };
  if (scope === "row") delete next.scope;
  else next.scope = "aggregate";
  return next;
}

/** Names a filter can use: dataset columns for row filters, measure outputs for totals. */
export function filterFieldOptions(scope: FilterScope, visual: Visual | undefined, columns: ColumnView[]): string[] {
  if (scope === "aggregate" && visual) return [...new Set(visual.measures.map(outputName))];
  return columns.map((c) => c.name);
}

/** The type family used to read typed values. A total takes the type of its measure. */
export function filterFamily(filter: Filter, visual: Visual | undefined, columns: ColumnView[]): Family | undefined {
  if (filterScope(filter) === "row" || !visual) return familyOf(columns, filter.field);
  const measure = visual.measures.find((m) => outputName(m) === filter.field);
  if (!measure) return undefined;
  if (measure.agg === "count" || measure.agg === "count_distinct" || measure.agg === "sum" || measure.agg === "avg") {
    return "number";
  }
  return familyOf(columns, measure.field);
}

/** One value row in the filter editor. Typed text, an empty (NULL) value, or a parameter. */
export type ValueDraft = { kind: "text"; text: string } | { kind: "null" } | { kind: "param"; name: string };

/** Reads a stored value. Anything unusual is shown as its JSON text, so nothing is dropped. */
export function valueDraft(value: unknown): ValueDraft {
  if (isParamRef(value)) return { kind: "param", name: value.param };
  if (value === null) return { kind: "null" };
  if (typeof value === "string") return { kind: "text", text: value };
  if (typeof value === "number" || typeof value === "boolean") return { kind: "text", text: String(value) };
  return { kind: "text", text: JSON.stringify(value) };
}

/** Parses typed text for a column family, as dv-core expects (numbers and booleans are converted). */
export function parseFilterValue(text: string, family: Family | undefined): unknown {
  const trimmed = text.trim();
  if (family === "number") {
    const n = Number(trimmed);
    return trimmed !== "" && Number.isFinite(n) ? n : trimmed;
  }
  if (family === "bool") return trimmed.toLowerCase() === "true";
  return trimmed;
}

export function valueFromDraft(draft: ValueDraft, family: Family | undefined): unknown {
  if (draft.kind === "param") return { param: draft.name };
  if (draft.kind === "null") return null;
  return parseFilterValue(draft.text, family);
}

/** The rows shown for a filter: its values, padded with blanks up to what its operator needs. */
export function valueDrafts(filter: Filter): ValueDraft[] {
  const drafts = filter.values.map(valueDraft);
  const slots = FILTER_OPS.find((o) => o.value === filter.op)?.slots ?? "list";
  const needed = slots === "two" ? 2 : slots === "none" ? 0 : 1;
  while (drafts.length < needed) drafts.push({ kind: "text", text: "" });
  return drafts;
}

// ---------------------------------------------------------------------------
// Row limit: a number, or a parameter
// ---------------------------------------------------------------------------

export type LimitDraft = { kind: "none" } | { kind: "number"; text: string } | { kind: "param"; name: string };

export function limitDraft(limit: Limit | null | undefined): LimitDraft {
  if (typeof limit === "number") return { kind: "number", text: String(limit) };
  if (isParamRef(limit)) return { kind: "param", name: limit.param };
  return { kind: "none" };
}

/** Numbers are at least 1, as before. Blank or non-numeric text means no limit. */
export function limitFromDraft(draft: LimitDraft): Limit | null {
  if (draft.kind === "param") return { param: draft.name };
  if (draft.kind === "none") return null;
  if (draft.text.trim() === "") return null;
  const n = Number(draft.text);
  return Number.isFinite(n) ? Math.max(1, Math.floor(n)) : null;
}

// ---------------------------------------------------------------------------
// Parameters: types, values, and checks that mirror dv-core
// ---------------------------------------------------------------------------

/** The declared type, or the type implied by the default (text when there is none). */
export function paramType(p: Parameter): ParamType {
  if (p.type) return p.type;
  if (typeof p.default === "boolean") return "boolean";
  if (typeof p.default === "number") return Number.isInteger(p.default) ? "integer" : "number";
  return "string";
}

/** Text shown for a parameter's default, or "" when it has none (it is then required). */
export function defaultText(p: Parameter): string {
  return p.default === undefined || p.default === null ? "" : String(p.default);
}

/** The text a parameter holds now: what the user entered, else its default. */
export function paramText(p: Parameter, values: Record<string, string>): string {
  const entered = Object.hasOwn(values, p.name) ? values[p.name] : undefined;
  return entered ?? defaultText(p);
}

/** The entries for declared parameters only. Parameters without an entry use their defaults. */
export function requestValues(parameters: Parameter[], values: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of parameters) {
    if (Object.hasOwn(values, p.name)) out[p.name] = values[p.name] ?? "";
  }
  return out;
}

const INTEGER_TEXT = /^[+-]?\d+$/;
const NUMBER_TEXT = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;
const DATE_TEXT = /^\d{4}-\d{2}-\d{2}$/;

function daysInMonth(year: number, month: number): number {
  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

/** True for YYYY-MM-DD that names a real day. */
export function validDate(text: string): boolean {
  if (!DATE_TEXT.test(text)) return false;
  const year = Number(text.slice(0, 4));
  const month = Number(text.slice(5, 7));
  const day = Number(text.slice(8, 10));
  return month >= 1 && month <= 12 && day >= 1 && day <= daysInMonth(year, month);
}

/** Why `text` is not an acceptable value for `p`, or null. Same rules as dv-core's resolve_parameters. */
export function paramError(p: Parameter, text: string): string | null {
  const kind = paramType(p);
  const numeric = kind === "number" || kind === "integer";
  if (kind === "boolean" && text !== "true" && text !== "false") return "Choose yes or no.";
  if (kind === "string" && text === "" && p.default === undefined) return "Enter a value.";
  if (kind === "number" && !NUMBER_TEXT.test(text)) return "Enter a number.";
  if (kind === "integer" && !INTEGER_TEXT.test(text)) return "Enter a whole number.";
  if (kind === "date" && !validDate(text)) return "Enter a date as YYYY-MM-DD.";

  if (p.allowed && p.allowed.length > 0) {
    const allowed = p.allowed.some((a) => (numeric ? Number(a) === Number(text) : String(a) === text));
    if (!allowed) return `Choose one of: ${p.allowed.map((a) => String(a)).join(", ")}.`;
  }
  if (p.min !== undefined && p.min !== null && (numeric ? Number(text) < Number(p.min) : text < String(p.min))) {
    return `Must be at least ${String(p.min)}.`;
  }
  if (p.max !== undefined && p.max !== null && (numeric ? Number(text) > Number(p.max) : text > String(p.max))) {
    return `Must be at most ${String(p.max)}.`;
  }
  return null;
}

/** Checks a parameter declaration as a whole (its default, allowed list and bounds). */
export function paramDeclError(p: Parameter): string | null {
  const kind = paramType(p);
  const ranged = kind === "number" || kind === "integer" || kind === "date";
  if ((p.min !== undefined || p.max !== undefined) && !ranged) {
    return "Min and max apply to number, whole number and date parameters only.";
  }
  if (p.min !== undefined && p.max !== undefined) {
    const inOrder =
      kind === "date" ? String(p.min) <= String(p.max) : Number(p.min) <= Number(p.max);
    if (!inOrder) return "Min is greater than max.";
  }
  if (p.default !== undefined && p.default !== null) {
    const d = p.default;
    const typeOk =
      (kind === "string" && typeof d === "string") ||
      (kind === "boolean" && typeof d === "boolean") ||
      (kind === "number" && typeof d === "number" && Number.isFinite(d)) ||
      (kind === "integer" && typeof d === "number" && Number.isSafeInteger(d)) ||
      (kind === "date" && typeof d === "string" && validDate(d));
    if (!typeOk) return `The default must be a ${PARAM_TYPES.find((t) => t.value === kind)?.label.toLowerCase()} value.`;
  }
  if (p.default !== undefined && p.default !== null) {
    const error = paramError(p, defaultText(p));
    if (error) return `The default is not allowed: ${error}`;
  }
  return null;
}

/** Problems with the entered values, keyed by parameter name. Empty when all are acceptable. */
export function parameterIssues(parameters: Parameter[], values: Record<string, string>): Record<string, string> {
  const issues: Record<string, string> = {};
  for (const p of parameters) {
    const error = paramError(p, paramText(p, values));
    if (error) issues[p.name] = error;
  }
  return issues;
}



/** Titles of visuals and dashboards whose filters or row limit refer to a parameter. */
export function parameterUsers(report: Report, name: string): string[] {
  const visuals = report.visuals
    .filter(
      (v) =>
        (isParamRef(v.limit) && v.limit.param === name) ||
        v.filters.some((f) => f.values.some((x) => isParamRef(x) && x.param === name)),
    )
    .map((v) => v.title || v.id);
  const dashboards = (report.dashboards ?? [])
    .filter((d) => (d.filters ?? []).some((f) => f.values.some((x) => isParamRef(x) && x.param === name)))
    .map((d) => `${d.title || d.id} (shared filter)`);
  return [...visuals, ...dashboards];
}

/**
 * Titles of visuals on `dataset` that use a field as a dimension, measure, encoding, filter or sort key,
 * and of dashboards with a shared filter on it that reaches one of those visuals.
 */
export function fieldUsers(report: Report, dataset: string, field: string): string[] {
  const visuals = report.visuals
    .filter(
      (v) =>
        v.dataset === dataset &&
        (v.dimensions.includes(field) ||
          v.measures.some((m) => m.field === field) ||
          Object.values(v.encodings).includes(field) ||
          v.filters.some((f) => f.field === field) ||
          v.sort?.by === field),
    )
    .map((v) => v.title || v.id);
  const dashboards = (report.dashboards ?? [])
    .filter(
      (d) =>
        (d.filters ?? []).some((f) => f.field === field) &&
        d.items.some((i) => report.visuals.some((v) => v.id === i.visual && v.dataset === dataset)),
    )
    .map((d) => `${d.title || d.id} (shared filter)`);
  return [...visuals, ...dashboards];
}

/** The report with a parameter added, or the one named `original` replaced. */
export function withParameter(report: Report, param: Parameter, original: string | null): Report {
  const index = original === null ? -1 : report.parameters.findIndex((p) => p.name === original);
  const parameters =
    index >= 0 ? report.parameters.map((p, i) => (i === index ? param : p)) : [...report.parameters, param];
  return { ...report, parameters };
}

export function withoutParameter(report: Report, name: string): Report {
  return { ...report, parameters: report.parameters.filter((p) => p.name !== name) };
}

// ---------------------------------------------------------------------------
// Calculated fields
// ---------------------------------------------------------------------------

/** Name rules for a calculated field, and no clash with a column or another field (case-insensitive). */
export function calcNameError(name: string, taken: string[]): string | null {
  if (name === "") return "Enter a name.";
  if (!/^[A-Za-z0-9_-]+$/.test(name)) return "Use only letters, digits, _ or -.";
  const key = name.toLowerCase();
  if (taken.some((t) => t.toLowerCase() === key)) return `"${name}" is already a column or calculated field.`;
  return null;
}

function withFields(dataset: Dataset, fields: CalculatedField[]): Dataset {
  const next: Dataset = { ...dataset };
  if (fields.length > 0) next.calculated_fields = fields;
  else delete next.calculated_fields;
  return next;
}

/** The report with a calculated field added to a dataset, or the one named `original` replaced. */
export function withCalculatedField(
  report: Report,
  dataset: string,
  field: CalculatedField,
  original: string | null,
): Report {
  return {
    ...report,
    datasets: report.datasets.map((d) => {
      if (d.name !== dataset) return d;
      const current = d.calculated_fields ?? [];
      const index = original === null ? -1 : current.findIndex((f) => f.name === original);
      const fields = index >= 0 ? current.map((f, i) => (i === index ? field : f)) : [...current, field];
      return withFields(d, fields);
    }),
  };
}

export function withoutCalculatedField(report: Report, dataset: string, name: string): Report {
  return {
    ...report,
    datasets: report.datasets.map((d) =>
      d.name === dataset ? withFields(d, (d.calculated_fields ?? []).filter((f) => f.name !== name)) : d,
    ),
  };
}

/** The dataset as it is after an edit, for re-registering it in the engine. */
export function datasetOf(report: Report, dataset: string): Dataset | undefined {
  return report.datasets.find((d) => d.name === dataset);
}

// ---------------------------------------------------------------------------
// Joined datasets: names, choices, building and describing a join, and which
// datasets must be registered again after a change (see docs/reports.md, Joins)
// ---------------------------------------------------------------------------

export const JOIN_KINDS: { value: JoinKind; label: string; sql: string }[] = [
  { value: "inner", label: "Inner - only rows that match in both", sql: "INNER" },
  { value: "left", label: "Left - all rows from the left dataset", sql: "LEFT" },
  { value: "right", label: "Right - all rows from the right dataset", sql: "RIGHT" },
  { value: "full", label: "Full - all rows from both", sql: "FULL" },
];

/** A join's kind. A missing kind is inner, as in dv-core. */
export function joinKindOf(join: Join): JoinKind {
  return join.kind ?? "inner";
}

/** Name rules for a joined dataset: the same shape as a calculated field, and unique among sources and datasets. */
export function joinNameError(name: string, takenNames: string[]): string | null {
  if (name === "") return "Enter a name.";
  if (!/^[A-Za-z0-9_-]+$/.test(name)) return "Use only letters, digits, _ or -.";
  const key = name.toLowerCase();
  if (takenNames.some((t) => t.toLowerCase() === key)) return `"${name}" is already a source or dataset.`;
  return null;
}

export interface JoinableRelation {
  name: string;
  columns: ColumnView[];
}

/**
 * Datasets in report order, with their columns. A join can only use datasets defined earlier,
 * so when `before` names an existing dataset, only the ones before it are returned. Without
 * `before` (a new dataset), all of them are.
 */
export function joinableRelations(
  report: Report,
  schemas: Record<string, ColumnView[]>,
  before?: string,
): JoinableRelation[] {
  const index = before === undefined ? -1 : report.datasets.findIndex((d) => d.name === before);
  const list = index >= 0 ? report.datasets.slice(0, index) : report.datasets;
  return list.map((d) => ({ name: d.name, columns: schemas[d.name] ?? [] }));
}

/**
 * Names of the datasets that read `name` (as their source or a join's `with`) and of the visuals
 * that use it. Empty when nothing uses it, so it can be deleted.
 */
export function datasetDependents(report: Report, name: string): string[] {
  const datasets = report.datasets
    .filter((d) => d.name !== name && (d.source === name || (d.joins ?? []).some((j) => j.with === name)))
    .map((d) => d.name);
  const visuals = report.visuals.filter((v) => v.dataset === name).map((v) => v.title || v.id);
  return [...datasets, ...visuals];
}

/** Checks the keys of a join: each needs a column on both sides, and no two keys are the same. */
export function joinKeysError(keys: JoinKey[]): string | null {
  if (keys.length === 0) return "Add at least one key.";
  if (keys.some((k) => k.left === "" || k.right === "")) return "Choose a column on both sides of every key.";
  const seen = new Set<string>();
  for (const k of keys) {
    const id = JSON.stringify([k.left, k.right]);
    if (seen.has(id)) return "Two keys are the same. Remove one of them.";
    seen.add(id);
  }
  return null;
}

/**
 * A joined dataset: `source` is the left dataset, with one join to `right`. When editing, `base`
 * keeps what the form does not show: later joins and calculated fields.
 */
export function buildJoinedDataset(
  name: string,
  left: string,
  right: string,
  kind: JoinKind,
  keys: JoinKey[],
  base: Dataset | null = null,
): Dataset {
  const join: Join = { with: right, kind, on: keys.map((k) => ({ left: k.left, right: k.right })) };
  const next: Dataset = { name, source: left, joins: [join, ...(base?.joins ?? []).slice(1)] };
  if (base?.calculated_fields && base.calculated_fields.length > 0) next.calculated_fields = base.calculated_fields;
  return next;
}

/** The report with a joined dataset appended. */
export function addJoinedDataset(report: Report, dataset: Dataset): Report {
  return { ...report, datasets: [...report.datasets, dataset] };
}

/** The report with the dataset named `original` replaced in place (its position is kept). */
export function replaceDataset(report: Report, original: string, dataset: Dataset): Report {
  return { ...report, datasets: report.datasets.map((d) => (d.name === original ? dataset : d)) };
}

/** The report without the dataset named `name`. Callers check datasetDependents first. */
export function removeDataset(report: Report, name: string): Report {
  return { ...report, datasets: report.datasets.filter((d) => d.name !== name) };
}

/** A short description of a dataset's joins, as SQL reads: "orders LEFT JOIN regions ON region = region". */
export function describeJoin(dataset: Dataset): string {
  const joins = dataset.joins ?? [];
  if (joins.length === 0) return "";
  let text = dataset.source;
  for (const join of joins) {
    const sql = JOIN_KINDS.find((k) => k.value === joinKindOf(join))?.sql ?? "INNER";
    const keys = join.on.map((k) => `${k.left} = ${k.right}`).join(" AND ");
    text += ` ${sql} JOIN ${join.with} ON ${keys}`;
  }
  return text;
}

/** What the engine's registration of a dataset depends on. A change here means it must be registered again. */
function definitionOf(d: Dataset): string {
  return JSON.stringify({
    source: d.source,
    sql: d.sql ?? null,
    joins: d.joins ?? [],
    calculated_fields: d.calculated_fields ?? [],
  });
}

/**
 * The datasets to register again after a change from `before` to `after` (undo, redo, or a save):
 * new datasets, datasets whose own definition changed, and datasets that read one of those.
 * They come in report order, which puts every dataset after the datasets it reads.
 */
export function datasetsToRefresh(before: Report, after: Report): Dataset[] {
  const previous = new Map(before.datasets.map((d) => [d.name, d]));
  const changed = new Set<string>();
  const out: Dataset[] = [];
  for (const d of after.datasets) {
    const prior = previous.get(d.name);
    const inputs = [d.source, ...(d.joins ?? []).map((j) => j.with)];
    if (!prior || definitionOf(prior) !== definitionOf(d) || inputs.some((i) => changed.has(i))) {
      changed.add(d.name);
      out.push(d);
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Dashboards: the report's list, and the edits that keep it valid
// ---------------------------------------------------------------------------

/** A dashboard id that no other dashboard uses. Ids follow the visual id rules. */
export function newDashboardId(report: Report): string {
  const used = new Set((report.dashboards ?? []).map((d) => d.id));
  let n = used.size + 1;
  while (used.has(`dashboard_${n}`)) n += 1;
  return `dashboard_${n}`;
}

/** An empty dashboard with the default grid, titled with its number. */
export function blankDashboard(report: Report): Dashboard {
  const id = newDashboardId(report);
  return {
    id,
    title: `Dashboard ${id.slice("dashboard_".length)}`,
    columns: GRID_LIMITS.defaultColumns,
    row_height: GRID_LIMITS.defaultRowHeight,
    items: [],
  };
}

/** The report with its dashboards replaced. The key is removed when the list is empty, so the file is unchanged. */
export function withDashboards(report: Report, dashboards: Dashboard[]): Report {
  const next: Report = { ...report };
  if (dashboards.length > 0) next.dashboards = dashboards;
  else delete next.dashboards;
  return next;
}

/** The report with `dashboard` added, or the dashboard with the same id replaced. */
export function replaceDashboard(report: Report, dashboard: Dashboard): Report {
  const list = report.dashboards ?? [];
  const exists = list.some((d) => d.id === dashboard.id);
  return withDashboards(report, exists ? list.map((d) => (d.id === dashboard.id ? dashboard : d)) : [...list, dashboard]);
}

export function withoutDashboard(report: Report, id: string): Report {
  return withDashboards(report, (report.dashboards ?? []).filter((d) => d.id !== id));
}

/** A dashboard's filters, with the key removed when there are none (so the file stays unchanged). */
export function withDashboardFilters(dashboard: Dashboard, filters: Filter[]): Dashboard {
  const next: Dashboard = { ...dashboard };
  if (filters.length > 0) next.filters = filters;
  else delete next.filters;
  return next;
}

/** Dashboards that place a visual. */
export function dashboardsWithVisual(report: Report, visualId: string): Dashboard[] {
  return (report.dashboards ?? []).filter((d) => d.items.some((i) => i.visual === visualId));
}

/** The report with every placement of a visual removed from every dashboard. */
export function withoutVisualPlacements(report: Report, visualId: string): Report {
  return withDashboards(
    report,
    (report.dashboards ?? []).map((d) => ({ ...d, items: d.items.filter((i) => i.visual !== visualId) })),
  );
}

/** Columns of a dashboard's visuals, one entry per name (the first dataset that has it). */
export function dashboardColumns(report: Report, dashboard: Dashboard, schemas: Record<string, ColumnView[]>): ColumnView[] {
  const seen = new Map<string, ColumnView>();
  for (const item of dashboard.items) {
    const visual = report.visuals.find((v) => v.id === item.visual);
    for (const column of visual ? schemas[visual.dataset] ?? [] : []) {
      if (!seen.has(column.name)) seen.set(column.name, column);
    }
  }
  return [...seen.values()];
}

import { useEffect, useState, type DragEvent, type SetStateAction } from "react";
import type { ChartType, ColumnView, Parameter, Visual } from "../types";
import {
  AGGREGATES,
  CHART_TYPES,
  SLOTS,
  limitDraft,
  limitFromDraft,
  outputNames,
  paramType,
  type LimitDraft,
  type Pill,
  type PillAgg,
  type Slot,
  type Slots,
} from "../model";
import { FIELD_MIME, type FieldTarget } from "./DataPanel";
import { FilterEditor } from "./FilterEditor";
import { ChartIcon } from "./icons";


/** Controls are at least 24px tall and text is at least 12px. */
const control =
  "min-h-6 rounded-md border border-border bg-surface px-2 py-0.5 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";

/** What each chart type needs on its shelves. Shown under the chart-type picker. */
export const CHART_HINTS: Record<ChartType, string> = {
  bar: "Bar: one category on Columns, one measure on Rows. Color adds a series.",
  line: "Line: one date or ordered dimension on Columns, one measure on Rows. Color adds a series.",
  scatter: "Scatter: one measure on Columns and one on Rows. Color and Size are optional.",
  table: "Table: each field on Columns or Rows becomes a column, with one row per result.",
  heatmap: "Heatmap: a dimension on Columns, another on Rows, and a measure on Color.",
};

/**
 * Why the chart would draw nothing with these shelves, or null when it can be drawn. The
 * message says what to add, in the same words the shelves use.
 */
export function emptyChartHint(chart: ChartType, slots: Slots): string | null {
  switch (chart) {
    case "bar":
    case "line":
      if (!slots.y) return "Add a measure to Rows to draw this chart.";
      if (!slots.x) return "Add a field to Columns to draw this chart.";
      return null;
    case "scatter":
      if (!slots.y) return "Add a measure to Rows to draw this chart.";
      if (!slots.x) return "Add a measure to Columns to draw this chart.";
      return null;
    case "heatmap":
      if (!slots.color) return "Add a measure to Color to draw this chart.";
      if (!slots.x || !slots.y) return "Add a field to Columns and Rows to draw this chart.";
      return null;
    case "table":
      return slots.x || slots.y ? null : "Add a field to Columns or Rows to draw this table.";
  }
}

/** Whether a field may be dropped on a shelf, and what the drop zone says about it. */
interface DropCheck {
  valid: boolean;
  message: string;
}

function slotCheck(chart: ChartType, slot: Slot, label: string): DropCheck {
  if (slot === "size" && chart !== "scatter") {
    return { valid: false, message: "Size is used by Scatter charts only. Choose Scatter to use it." };
  }
  return { valid: true, message: `Drop to place on ${label}.` };
}

function readDrop(event: DragEvent): ColumnView | null {
  const raw = event.dataTransfer.getData(FIELD_MIME);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as ColumnView;
  } catch {
    return null;
  }
}

/** Drop-target behaviour shared by the slots and the filter shelf. Invalid drops are refused. */
function dropProps(
  check: DropCheck,
  onDrop: (column: ColumnView) => void,
  setDrag: (update: SetStateAction<DropCheck | null>) => void,
) {
  return {
    onDragOver: (event: DragEvent) => {
      if (!event.dataTransfer.types.includes(FIELD_MIME)) return;
      if (check.valid) {
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
      } else {
        event.dataTransfer.dropEffect = "none";
      }
      setDrag((prev) => (prev && prev.valid === check.valid && prev.message === check.message ? prev : check));
    },
    onDragLeave: () => setDrag(null),
    onDrop: (event: DragEvent) => {
      event.preventDefault();
      setDrag(null);
      if (!check.valid) return;
      const column = readDrop(event);
      if (column) onDrop(column);
    },
  };
}

function dropClass(drag: DropCheck | null): string {
  if (!drag) return "border-border bg-surface";
  return drag.valid ? "border-accent bg-accent/10" : "border-danger bg-danger-bg";
}

interface ShelfProps {
  visual: Visual;
  slots: Slots;
  onSlot: (slot: Slot, pill: Pill | null) => void;
  onFieldDrop: (column: ColumnView, target: FieldTarget) => void;
}

/** The shelves for the given slots, in the order the slots are defined. */
function SlotShelves({ keys, visual, slots, onSlot, onFieldDrop }: ShelfProps & { keys: Slot[] }) {
  const emptyHint = emptyChartHint(visual.chart, slots);
  return (
    <>
      {SLOTS.filter((slot) => keys.includes(slot.key)).map((slot) => (
        <ShelfSlot
          key={slot.key}
          id={`shelf-${slot.key}`}
          label={slot.label}
          hint={slot.hint}
          pill={slots[slot.key]}
          check={slotCheck(visual.chart, slot.key, slot.label)}
          describedBy={emptyHint ? "shelves-empty-hint" : undefined}
          onDrop={(column) => onFieldDrop(column, slot.key)}
          onChange={(pill) => onSlot(slot.key, pill)}
        />
      ))}
    </>
  );
}

/** Columns and Rows as two full-width strips above the chart, the way Tableau lays them out. */
export function AxisShelves(props: ShelfProps) {
  return (
    <div className="flex flex-col gap-1.5">
      <SlotShelves keys={["x", "y"]} {...props} />
    </div>
  );
}

/** Why the chart is empty, in the shelves' own words. Each shelf points at it with aria-describedby. */
export function ShelvesHint({ visual, slots }: { visual: Visual; slots: Slots }) {
  const emptyHint = emptyChartHint(visual.chart, slots);
  if (!emptyHint) return null;
  return (
    <p id="shelves-empty-hint" role="status" className="rounded-md bg-surface-2 px-2 py-1 text-sm text-text">
      {emptyHint}
    </p>
  );
}

/** Row filters, for the cards column. */
export function FiltersCard({
  visual,
  columns,
  parameters,
  onChange,
  onFieldDrop,
}: {
  visual: Visual;
  columns: ColumnView[];
  parameters: Parameter[];
  onChange: (patch: Partial<Visual>) => void;
  onFieldDrop: (column: ColumnView, target: FieldTarget) => void;
}) {
  return (
    <FiltersShelf
      visual={visual}
      columns={columns}
      parameters={parameters}
      onChange={onChange}
      onDrop={(column) => onFieldDrop(column, "filter")}
    />
  );
}

/** Chart type, Color, Size, sort and row limit: everything that shapes the marks, for the cards column. */
export function MarksCard({
  visual,
  slots,
  parameters,
  onSlot,
  onFieldDrop,
  onChange,
}: ShelfProps & { parameters: Parameter[]; onChange: (patch: Partial<Visual>) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <fieldset className="flex flex-col gap-1">
        <legend className="text-xs font-semibold uppercase tracking-wide text-muted">Chart type</legend>
        <div
          role="radiogroup"
          aria-label="Chart type"
          aria-describedby="chart-type-hint"
          className="grid grid-cols-3 gap-1"
        >
          {CHART_TYPES.map((chart) => {
            const checked = visual.chart === chart.value;
            return (
              <label
                key={chart.value}
                className={`flex min-h-7 cursor-pointer flex-col items-center justify-center rounded-md border px-1 py-0.5 text-center text-xs font-medium focus-within:outline-2 focus-within:outline-accent ${
                  checked ? "border-accent bg-accent/15" : "border-border bg-surface hover:bg-surface-2"
                }`}
              >
                <input
                  type="radio"
                  name="chart-type"
                  value={chart.value}
                  checked={checked}
                  onChange={() => onChange({ chart: chart.value as ChartType })}
                  className="sr-only"
                />
                <ChartIcon chart={chart.value as ChartType} />
                {chart.label}
              </label>
            );
          })}
        </div>
        <p id="chart-type-hint" className="text-xs text-muted">
          {CHART_HINTS[visual.chart]}
        </p>
      </fieldset>

      <div className="flex flex-col gap-1.5">
        <SlotShelves keys={["color", "size"]} visual={visual} slots={slots} onSlot={onSlot} onFieldDrop={onFieldDrop} />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <label className="flex flex-col gap-1 text-xs font-semibold uppercase tracking-wide text-muted">
          Sort by
          <select
            className={`${control} normal-case tracking-normal`}
            value={visual.sort?.by ?? ""}
            onChange={(event) => {
              const by = event.target.value;
              onChange({ sort: by ? { by, direction: visual.sort?.direction ?? "desc" } : null });
            }}
          >
            <option value="">None</option>
            {outputNames(visual).map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs font-semibold uppercase tracking-wide text-muted">
          Direction
          <select
            className={`${control} normal-case tracking-normal`}
            value={visual.sort?.direction ?? "desc"}
            disabled={!visual.sort}
            onChange={(event) =>
              visual.sort && onChange({ sort: { by: visual.sort.by, direction: event.target.value as "asc" | "desc" } })
            }
          >
            <option value="desc">Descending</option>
            <option value="asc">Ascending</option>
          </select>
        </label>
        <LimitField limit={visual.limit} parameters={parameters} onChange={(limit) => onChange({ limit })} />
      </div>
    </div>
  );
}

/**
 * One shelf. Its hint and drop explanation are real text, linked to the section with
 * aria-describedby. While a field is dragged over it the zone shows whether it is valid.
 */
function ShelfSlot({
  id,
  label,
  hint,
  pill,
  check,
  describedBy,
  onDrop,
  onChange,
}: {
  id: string;
  label: string;
  hint: string;
  pill: Pill | null;
  check: DropCheck;
  describedBy?: string;
  onDrop: (column: ColumnView) => void;
  onChange: (pill: Pill | null) => void;
}) {
  const [drag, setDrag] = useState<DropCheck | null>(null);
  const hintId = `${id}-hint`;
  const bodyId = `${id}-body`;
  const showBody = drag !== null || pill === null;
  const described = [hintId, showBody ? bodyId : null, describedBy ?? null].filter(Boolean).join(" ");
  return (
    <section
      aria-label={`${label} shelf`}
      aria-describedby={described}
      {...dropProps(check, onDrop, setDrag)}
      className={`flex min-h-10 flex-wrap items-center gap-x-2 gap-y-1 rounded-md border-2 border-dashed px-2 py-1.5 ${dropClass(drag)}`}
    >
      <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">{label}</h3>
      <span id={hintId} className="text-xs text-muted">
        {hint}
      </span>
      {pill && <PillView label={label} pill={pill} onChange={onChange} />}
      {showBody && (
        <p id={bodyId} className={`text-xs ${drag && !drag.valid ? "font-medium text-danger" : "text-muted"}`}>
          {drag ? drag.message : "Drop a field here"}
        </p>
      )}
    </section>
  );
}

function PillView({ label, pill, onChange }: { label: string; pill: Pill; onChange: (pill: Pill | null) => void }) {
  const id = `${label.replace(/\s+/g, "-").toLowerCase()}-${pill.field}`.replace(/[^A-Za-z0-9_-]/g, "_");
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2 rounded-md bg-surface-2 px-2 py-0.5">
      <span className="min-w-0 truncate text-sm font-medium" title={pill.field}>
        {pill.field}
      </span>
      <label className="sr-only" htmlFor={`${id}-kind`}>
        Field kind for {pill.field} on {label}
      </label>
      <select
        id={`${id}-kind`}
        className={`${control} text-xs`}
        value={pill.kind}
        onChange={(event) => {
          const kind = event.target.value as Pill["kind"];
          onChange({ field: pill.field, kind, agg: kind === "measure" ? "sum" : pill.agg });
        }}
      >
        <option value="dimension">Dimension</option>
        <option value="measure">Measure</option>
      </select>
      {pill.kind === "measure" && (
        <>
          <label className="sr-only" htmlFor={`${id}-agg`}>
            Aggregation for {pill.field} on {label}
          </label>
          <select
            id={`${id}-agg`}
            className={`${control} text-xs`}
            value={pill.agg}
            onChange={(event) => onChange({ ...pill, agg: event.target.value as PillAgg })}
          >
            {AGGREGATES.map((agg) => (
              <option key={agg.value} value={agg.value}>
                {agg.label}
              </option>
            ))}
          </select>
        </>
      )}
      <button
        type="button"
        onClick={() => onChange(null)}
        aria-label={`Remove ${pill.field} from ${label}`}
        className="ml-auto min-h-6 rounded px-2 py-0.5 text-sm text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        Remove
      </button>
    </div>
  );
}

function FiltersShelf({
  visual,
  columns,
  parameters,
  onChange,
  onDrop,
}: {
  visual: Visual;
  columns: ColumnView[];
  parameters: Parameter[];
  onChange: (patch: Partial<Visual>) => void;
  onDrop: (column: ColumnView) => void;
}) {
  const [drag, setDrag] = useState<DropCheck | null>(null);
  const valid: DropCheck = { valid: true, message: "Drop to add a row filter on this field." };
  return (
    <section
      aria-label="Filters shelf"
      aria-describedby="filters-shelf-hint"
      {...dropProps(valid, onDrop, setDrag)}
      className={`flex flex-col gap-2 rounded-md border-2 border-dashed p-2 ${dropClass(drag)}`}
    >
      <p id="filters-shelf-hint" className="text-xs text-muted">
        {drag ? drag.message : "Filters keep rows that match. Totals filter measures after grouping."}
      </p>
      <FilterEditor
        title="Filters"
        filters={visual.filters}
        columns={columns}
        parameters={parameters}
        visual={visual}
        onChange={(filters) => onChange({ filters })}
      />
    </section>
  );
}

/**
 * Row limit: no limit, a number, or a whole-number parameter. The number stays
 * selected while the box is empty (an empty box means no limit).
 */
function LimitField({
  limit,
  parameters,
  onChange,
}: {
  limit: Visual["limit"];
  parameters: Parameter[];
  onChange: (limit: Visual["limit"]) => void;
}) {
  const draft: LimitDraft = limitDraft(limit);
  const [numberMode, setNumberMode] = useState(draft.kind === "number");
  useEffect(() => {
    if (draft.kind !== "none") setNumberMode(draft.kind === "number");
  }, [draft.kind]);

  const integerParams = parameters.filter((p) => paramType(p) === "integer");
  const sourceValue = draft.kind === "param" ? `param:${draft.name}` : numberMode ? "number" : "none";
  const referenced = draft.kind === "param" && !integerParams.some((p) => p.name === draft.name);

  return (
    <div role="group" aria-labelledby="row-limit-label" className="col-span-2 flex flex-col gap-1">
      <span id="row-limit-label" className="text-xs font-semibold uppercase tracking-wide text-muted">
        Row limit
      </span>
      <div className="flex flex-wrap items-center gap-2">
        <label className="sr-only" htmlFor="row-limit-source">
          Row limit source
        </label>
        <select
          id="row-limit-source"
          className={`${control} normal-case tracking-normal`}
          value={sourceValue}
          onChange={(e) => {
            const value = e.target.value;
            if (value === "none") {
              setNumberMode(false);
              onChange(null);
            } else if (value === "number") {
              setNumberMode(true);
              onChange(limitFromDraft({ kind: "number", text: draft.kind === "number" ? draft.text : "" }));
            } else {
              setNumberMode(false);
              onChange({ param: value.slice("param:".length) });
            }
          }}
        >
          <option value="none">No limit</option>
          <option value="number">Number of rows</option>
          {integerParams.map((p) => (
            <option key={p.name} value={`param:${p.name}`}>
              Parameter: {p.name}
            </option>
          ))}
          {referenced && draft.kind === "param" && (
            <option value={sourceValue}>Parameter: {draft.name} (not a whole number)</option>
          )}
        </select>
        {sourceValue === "number" && (
          <input
            type="number"
            min={1}
            aria-label="Row limit"
            placeholder="Rows"
            className={`${control} normal-case tracking-normal w-28`}
            value={draft.kind === "number" ? draft.text : ""}
            onChange={(event) => onChange(limitFromDraft({ kind: "number", text: event.target.value }))}
          />
        )}
      </div>
    </div>
  );
}

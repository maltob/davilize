import { useEffect, useState, type DragEvent } from "react";
import type { ChartType, ColumnView, Filter, FilterOp, FilterScope, Parameter, Visual } from "../types";
import {
  AGGREGATES,
  CHART_TYPES,
  FILTER_OPS,
  FILTER_SCOPES,
  SLOTS,
  filterFamily,
  filterFieldOptions,
  filterScope,
  limitDraft,
  limitFromDraft,
  outputNames,
  paramType,
  valueDraft,
  valueDrafts,
  valueFromDraft,
  withScope,
  type LimitDraft,
  type Pill,
  type PillAgg,
  type Slot,
  type Slots,
  type ValueDraft,
} from "../model";
import { FIELD_MIME, type FieldTarget } from "./DataPanel";
import { ChartIcon } from "./icons";

interface Props {
  visual: Visual;
  slots: Slots;
  columns: ColumnView[];
  parameters: Parameter[];
  onSlot: (slot: Slot, pill: Pill | null) => void;
  onFieldDrop: (column: ColumnView, target: FieldTarget) => void;
  onChange: (patch: Partial<Visual>) => void;
}

const control =
  "rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";

function readDrop(event: DragEvent): ColumnView | null {
  const raw = event.dataTransfer.getData(FIELD_MIME);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as ColumnView;
  } catch {
    return null;
  }
}

/** Drop-target behaviour shared by the slots and the filter shelf. */
function dropProps(onDrop: (column: ColumnView) => void, setOver: (over: boolean) => void) {
  return {
    onDragOver: (event: DragEvent) => {
      if (event.dataTransfer.types.includes(FIELD_MIME)) {
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
        setOver(true);
      }
    },
    onDragLeave: () => setOver(false),
    onDrop: (event: DragEvent) => {
      event.preventDefault();
      setOver(false);
      const column = readDrop(event);
      if (column) onDrop(column);
    },
  };
}

export function Shelves({ visual, slots, columns, parameters, onSlot, onFieldDrop, onChange }: Props) {
  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
        {SLOTS.map((slot) => (
          <ShelfSlot
            key={slot.key}
            label={slot.label}
            hint={slot.hint}
            pill={slots[slot.key]}
            onDrop={(column) => onFieldDrop(column, slot.key)}
            onChange={(pill) => onSlot(slot.key, pill)}
          />
        ))}
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[1fr_20rem]">
        <FiltersShelf
          visual={visual}
          columns={columns}
          parameters={parameters}
          onChange={onChange}
          onDrop={(column) => onFieldDrop(column, "filter")}
        />
        <div className="flex flex-col gap-3">
          <fieldset className="flex flex-col gap-1">
            <legend className="text-xs font-semibold uppercase tracking-wide text-muted">Chart type</legend>
            <div role="radiogroup" aria-label="Chart type" className="grid grid-cols-5 gap-1">
              {CHART_TYPES.map((chart) => {
                const checked = visual.chart === chart.value;
                return (
                  <label
                    key={chart.value}
                    className={`cursor-pointer rounded-md border px-1 py-1 text-center text-xs font-medium focus-within:outline-2 focus-within:outline-accent ${
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
          </fieldset>
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
                  visual.sort &&
                  onChange({ sort: { by: visual.sort.by, direction: event.target.value as "asc" | "desc" } })
                }
              >
                <option value="desc">Descending</option>
                <option value="asc">Ascending</option>
              </select>
            </label>
            <LimitField
              limit={visual.limit}
              parameters={parameters}
              onChange={(limit) => onChange({ limit })}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

function ShelfSlot({
  label,
  hint,
  pill,
  onDrop,
  onChange,
}: {
  label: string;
  hint: string;
  pill: Pill | null;
  onDrop: (column: ColumnView) => void;
  onChange: (pill: Pill | null) => void;
}) {
  const [over, setOver] = useState(false);
  return (
    <section
      aria-label={`${label} shelf`}
      {...dropProps(onDrop, setOver)}
      className={`flex min-h-16 flex-col gap-1 rounded-md border-2 border-dashed p-2 ${
        over ? "border-accent bg-accent/10" : "border-border bg-surface"
      }`}
    >
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">{label}</h3>
        <span className="text-xs text-muted">{hint}</span>
      </div>
      {pill ? <PillView label={label} pill={pill} onChange={onChange} /> : <p className="text-sm text-muted">Drop a field here, or use Add to on the field list.</p>}
    </section>
  );
}

function PillView({ label, pill, onChange }: { label: string; pill: Pill; onChange: (pill: Pill | null) => void }) {
  const id = `${label.replace(/\s+/g, "-").toLowerCase()}-${pill.field}`.replace(/[^A-Za-z0-9_-]/g, "_");
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md bg-surface-2 px-2 py-1">
      <span className="min-w-0 truncate font-medium" title={pill.field}>
        {pill.field}
      </span>
      <label className="sr-only" htmlFor={`${id}-kind`}>
        Field kind for {pill.field} on {label}
      </label>
      <select
        id={`${id}-kind`}
        className={`${control} py-0.5 text-xs`}
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
            className={`${control} py-0.5 text-xs`}
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
        className="ml-auto rounded px-2 py-0.5 text-sm text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
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
  const [over, setOver] = useState(false);
  const setFilters = (filters: Filter[]) => onChange({ filters });
  const replace = (index: number, next: Filter) =>
    setFilters(visual.filters.map((f, i) => (i === index ? next : f)));
  const first = columns[0];

  return (
    <section
      aria-label="Filters shelf"
      {...dropProps(onDrop, setOver)}
      className={`flex flex-col gap-2 rounded-md border-2 border-dashed p-2 ${
        over ? "border-accent bg-accent/10" : "border-border bg-surface"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted">Filters</h3>
        <button
          type="button"
          onClick={() => first && setFilters([...visual.filters, { field: first.name, op: "is_not_null", values: [] }])}
          disabled={!first}
          className="rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface-2 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          Add filter
        </button>
      </div>
      {visual.filters.length === 0 && <p className="text-sm text-muted">No filters.</p>}
      {visual.filters.map((filter, index) => (
        <FilterRow
          key={index}
          index={index}
          filter={filter}
          visual={visual}
          columns={columns}
          parameters={parameters}
          onChange={(next) => replace(index, next)}
          onRemove={() => setFilters(visual.filters.filter((_, i) => i !== index))}
        />
      ))}
      {parameters.length === 0 && (
        <p className="text-xs text-muted">Declare a parameter in the Parameters panel to filter by a runtime value.</p>
      )}
    </section>
  );
}

/** One filter: its scope, field, condition and values. */
function FilterRow({
  index,
  filter,
  visual,
  columns,
  parameters,
  onChange,
  onRemove,
}: {
  index: number;
  filter: Filter;
  visual: Visual;
  columns: ColumnView[];
  parameters: Parameter[];
  onChange: (next: Filter) => void;
  onRemove: () => void;
}) {
  const scope = filterScope(filter);
  const options = filterFieldOptions(scope, visual, columns);
  // A field that is not offered here (for example from a hand-written file) stays visible.
  const fieldChoices = options.includes(filter.field) ? options : [filter.field, ...options];
  const family = filterFamily(filter, visual, columns);
  const slots = FILTER_OPS.find((o) => o.value === filter.op)?.slots ?? "list";
  const drafts = valueDrafts(filter);
  const writeValues = (next: ValueDraft[]) => onChange({ ...filter, values: next.map((d) => valueFromDraft(d, family)) });

  const changeScope = (next: FilterScope) => {
    const nextOptions = filterFieldOptions(next, visual, columns);
    const field = nextOptions.includes(filter.field) ? filter.field : (nextOptions[0] ?? filter.field);
    onChange({ ...withScope(filter, next), field });
  };

  const changeOp = (op: FilterOp) => {
    const nextSlots = FILTER_OPS.find((o) => o.value === op)?.slots ?? "list";
    const keep = nextSlots === "none" ? 0 : nextSlots === "one" ? 1 : nextSlots === "two" ? 2 : undefined;
    const values = filter.values.map(valueDraft).slice(0, keep);
    onChange({ ...filter, op, values: values.map((d) => valueFromDraft(d, family)) });
  };

  return (
    <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
      <div className="flex flex-wrap items-center gap-2">
        <label className="sr-only" htmlFor={`filter-scope-${index}`}>
          Filter {index + 1} applies to
        </label>
        <select
          id={`filter-scope-${index}`}
          className={`${control} py-0.5`}
          value={scope}
          onChange={(e) => changeScope(e.target.value as FilterScope)}
        >
          {FILTER_SCOPES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
        <label className="sr-only" htmlFor={`filter-field-${index}`}>
          Filter field
        </label>
        <select
          id={`filter-field-${index}`}
          className={`${control} py-0.5`}
          value={filter.field}
          onChange={(e) => onChange({ ...filter, field: e.target.value })}
        >
          {fieldChoices.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        <label className="sr-only" htmlFor={`filter-op-${index}`}>
          Filter condition
        </label>
        <select
          id={`filter-op-${index}`}
          className={`${control} py-0.5`}
          value={filter.op}
          onChange={(e) => changeOp(e.target.value as FilterOp)}
        >
          {FILTER_OPS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        {scope === "aggregate" && options.length === 0 && (
          <span className="text-xs text-muted">Add a measure to filter totals.</span>
        )}
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove filter on ${filter.field}`}
          className="ml-auto rounded px-2 py-0.5 text-sm text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          Remove
        </button>
      </div>

      {slots !== "none" &&
        drafts.map((draft, i) => (
          <ValueRow
            key={i}
            id={`filter-${index}-value-${i}`}
            label={slots === "two" ? (i === 0 ? "Lower" : "Upper") : slots === "list" ? `Value ${i + 1}` : "Value"}
            draft={draft}
            parameters={parameters}
            numeric={family === "number"}
            onChange={(next) => writeValues(drafts.map((d, j) => (j === i ? next : d)))}
            onRemove={slots === "list" && drafts.length > 1 ? () => writeValues(drafts.filter((_, j) => j !== i)) : undefined}
          />
        ))}
      {slots === "list" && (
        <button
          type="button"
          onClick={() => writeValues([...drafts, { kind: "text", text: "" }])}
          className="self-start rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface focus-visible:outline-2 focus-visible:outline-accent"
        >
          Add value
        </button>
      )}
    </div>
  );
}

/** One value of a filter: typed text, empty (NULL), or a declared parameter. */
function ValueRow({
  id,
  label,
  draft,
  parameters,
  numeric,
  onChange,
  onRemove,
}: {
  id: string;
  label: string;
  draft: ValueDraft;
  parameters: Parameter[];
  numeric: boolean;
  onChange: (next: ValueDraft) => void;
  onRemove?: () => void;
}) {
  const storedText = draft.kind === "text" ? draft.text : null;
  const [text, setText] = useState(storedText ?? "");
  // Follow edits made elsewhere (a removed filter shifts rows), but keep typing in progress.
  useEffect(() => {
    if (storedText !== null) setText(storedText);
  }, [storedText]);

  const sourceValue = draft.kind === "param" ? `param:${draft.name}` : draft.kind === "null" ? "null" : "text";
  const declared = draft.kind === "param" && parameters.some((p) => p.name === draft.name);
  const commit = () => {
    if (draft.kind === "text" && text !== draft.text) onChange({ kind: "text", text });
  };

  return (
    <div className="flex flex-wrap items-center gap-2 pl-2">
      <label htmlFor={`${id}-source`} className="text-xs text-muted">
        {label}
      </label>
      <select
        id={`${id}-source`}
        className={`${control} py-0.5`}
        value={sourceValue}
        onChange={(e) => {
          const value = e.target.value;
          if (value === "text") onChange({ kind: "text", text: "" });
          else if (value === "null") onChange({ kind: "null" });
          else onChange({ kind: "param", name: value.slice("param:".length) });
        }}
      >
        <option value="text">Typed value</option>
        <option value="null">Empty (NULL)</option>
        {parameters.map((p) => (
          <option key={p.name} value={`param:${p.name}`}>
            Parameter: {p.name}
          </option>
        ))}
        {draft.kind === "param" && !declared && (
          <option value={sourceValue}>Parameter: {draft.name} (not declared)</option>
        )}
      </select>
      {draft.kind === "text" && (
        <input
          id={id}
          type="text"
          inputMode={numeric ? "decimal" : undefined}
          aria-label={`${label} value`}
          placeholder={numeric ? "number" : "value"}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
          className={`${control} w-36 py-0.5`}
        />
      )}
      {onRemove && (
        <button
          type="button"
          onClick={onRemove}
          aria-label={`Remove ${label.toLowerCase()}`}
          className="rounded px-2 py-0.5 text-xs text-muted hover:bg-surface hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
        >
          Remove value
        </button>
      )}
    </div>
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

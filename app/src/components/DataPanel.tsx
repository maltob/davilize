import type { CalculatedField, ColumnView, Dataset, Parameter, Report, Source } from "../types";
import { SLOTS, type Slot } from "../model";
import { CalculatedFieldsSection } from "./CalculatedFields";
import { ParametersSection } from "./Parameters";
import { TypeBadge } from "./icons";

/** MIME type for field drags. The payload is the field's ColumnView as JSON. */
export const FIELD_MIME = "application/x-davilize-field";
export type FieldTarget = Slot | "filter";

interface Props {
  report: Report;
  reportPath: string | null;
  datasets: Dataset[];
  sources: Source[];
  schemas: Record<string, ColumnView[]>;
  active: string | null;
  busy: boolean;
  onSelect: (dataset: string) => void;
  onAddFiles: () => void;
  onAddField: (column: ColumnView, target: FieldTarget) => void;
  /** Each resolves to an error message, or null when the change was applied. */
  onSaveCalculated: (dataset: string, field: CalculatedField, original: string | null) => Promise<string | null>;
  onDeleteCalculated: (dataset: string, name: string) => Promise<string | null>;
  onSaveParameter: (param: Parameter, original: string | null) => void;
  onDeleteParameter: (name: string) => void;
}

export function DataPanel({
  report,
  reportPath,
  datasets,
  sources,
  schemas,
  active,
  busy,
  onSelect,
  onAddFiles,
  onAddField,
  onSaveCalculated,
  onDeleteCalculated,
  onSaveParameter,
  onDeleteParameter,
}: Props) {
  const columns = active ? schemas[active] ?? [] : [];
  const dimensions = columns.filter((c) => c.role === "dimension");
  const measures = columns.filter((c) => c.role === "measure");

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-3">
      <section aria-labelledby="sources-heading" className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <h2 id="sources-heading" className="text-xs font-semibold uppercase tracking-wide text-muted">
            Data
          </h2>
          <button
            type="button"
            onClick={onAddFiles}
            disabled={busy}
            className="rounded-md border border-border bg-surface px-2 py-1 text-xs font-medium hover:bg-surface-2 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
          >
            Add data files
          </button>
        </div>
        {datasets.length === 0 ? (
          <p className="text-sm text-muted">
            No data yet. Add a CSV, Parquet, JSON or Arrow file, or open a report.
          </p>
        ) : (
          <ul className="flex flex-col gap-1" aria-label="Datasets">
            {datasets.map((dataset) => {
              const source = sources.find((s) => s.name === dataset.source);
              const pressed = dataset.name === active;
              return (
                <li key={dataset.name}>
                  <button
                    type="button"
                    aria-pressed={pressed}
                    onClick={() => onSelect(dataset.name)}
                    className={`flex w-full flex-col rounded-md px-2 py-1.5 text-left focus-visible:outline-2 focus-visible:outline-accent ${
                      pressed ? "bg-accent/15 text-text" : "hover:bg-surface-2"
                    }`}
                  >
                    <span className="truncate text-sm font-medium">{dataset.name}</span>
                    <span className="truncate text-xs text-muted">
                      {source?.type === "file" ? source.path.split(/[\/]/).pop() : dataset.source}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {active && (
        <>
          <FieldGroup title="Dimensions" columns={dimensions} onAddField={onAddField} />
          <FieldGroup title="Measures" columns={measures} onAddField={onAddField} />
          <p className="text-xs text-muted">
            Drag a field onto a shelf, or use the Add to menu. Numbers start as measures; change the kind on the shelf.
          </p>
          <CalculatedFieldsSection
            report={report}
            reportPath={reportPath}
            dataset={active}
            columns={columns}
            onSave={(field, original) => onSaveCalculated(active, field, original)}
            onDelete={(name) => onDeleteCalculated(active, name)}
          />
        </>
      )}
      <ParametersSection report={report} onSave={onSaveParameter} onDelete={onDeleteParameter} />
    </div>
  );
}

function FieldGroup({
  title,
  columns,
  onAddField,
}: {
  title: string;
  columns: ColumnView[];
  onAddField: (column: ColumnView, target: FieldTarget) => void;
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">
        {title} <span className="font-normal">({columns.length})</span>
      </h2>
      {columns.length === 0 ? (
        <p className="px-2 text-sm text-muted">None</p>
      ) : (
        <ul className="flex flex-col">
          {columns.map((column) => (
            <li key={column.name} className="flex items-center gap-2 rounded px-1 py-0.5 hover:bg-surface-2">
              <span
                draggable
                onDragStart={(event) => {
                  event.dataTransfer.setData(FIELD_MIME, JSON.stringify(column));
                  event.dataTransfer.effectAllowed = "copy";
                }}
                className="flex min-w-0 flex-1 cursor-grab items-center gap-2"
                title={`${column.name} (${column.data_type})`}
              >
                <TypeBadge family={column.family} role={column.role} />
                <span className="truncate text-sm">{column.name}</span>
                {column.calculated && (
                  <>
                    <span
                      aria-hidden="true"
                      title="Calculated field"
                      className="shrink-0 rounded bg-accent/15 px-1 font-mono text-[10px] font-semibold text-accent"
                    >
                      fx
                    </span>
                    <span className="sr-only">(calculated field)</span>
                  </>
                )}
              </span>
              <select
                aria-label={`Add ${column.name} to a shelf`}
                value=""
                onChange={(event) => {
                  const target = event.target.value as FieldTarget | "";
                  if (target) onAddField(column, target);
                }}
                className="max-w-28 rounded border border-border bg-surface px-1 py-0.5 text-xs focus-visible:outline-2 focus-visible:outline-accent"
              >
                <option value="">Add to...</option>
                {SLOTS.map((slot) => (
                  <option key={slot.key} value={slot.key}>
                    {slot.label}
                  </option>
                ))}
                <option value="filter">Filters</option>
              </select>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

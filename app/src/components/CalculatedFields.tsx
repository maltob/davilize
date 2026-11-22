import { useEffect, useRef, useState } from "react";
import { api, toApiError } from "../api";
import { calcNameError, fieldUsers, withCalculatedField } from "../model";
import type { CalculatedField, ColumnView, Report } from "../types";
import { SectionToggle } from "./icons";

const control =
  "min-h-7 w-full rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";
const small =
  "min-h-7 whitespace-nowrap rounded-md border border-border bg-surface px-2 text-xs font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";

interface Props {
  report: Report;
  reportPath: string | null;
  dataset: string;
  columns: ColumnView[];
  /** Resolves to an error message, or null when the field was saved. */
  onSave: (field: CalculatedField, original: string | null) => Promise<string | null>;
  /** Resolves to an error message, or null when the field was removed. */
  onDelete: (name: string) => Promise<string | null>;
}

/** Calculated fields of one dataset: list, add, edit and delete. */
export function CalculatedFieldsSection({ report, reportPath, dataset, columns, onSave, onDelete }: Props) {
  const fields = report.datasets.find((d) => d.name === dataset)?.calculated_fields ?? [];
  // `null` means the form is closed; `original` is the name being edited, or null for a new field.
  const [editing, setEditing] = useState<{ original: string | null } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(true);

  const remove = async (name: string) => {
    setError(null);
    const message = await onDelete(name);
    if (message) setError(message);
  };

  // Adding a field always shows the section, even when it was collapsed.
  const startNew = () => {
    setOpen(true);
    setEditing({ original: null });
  };

  return (
    <section aria-labelledby="calc-heading" className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <SectionToggle
          title="Calculated fields"
          count={fields.length}
          open={open}
          controls="calc-section"
          headingId="calc-heading"
          onToggle={() => setOpen((value) => !value)}
        />
        <button type="button" className={small} onClick={startNew} disabled={editing !== null}>
          Add calculated field
        </button>
      </div>

      <div id="calc-section" hidden={!open} className="space-y-1">
      {fields.length === 0 && editing === null && (
        <p className="px-2 text-xs text-muted">Example: revenue * 0.9</p>
      )}

      {fields.length > 0 && (
        <ul className="flex flex-col gap-1" aria-label={`Calculated fields of ${dataset}`}>
          {fields.map((field) => {
            const used = fieldUsers(report, dataset, field.name);
            return (
              <li key={field.name} className="flex flex-col gap-0.5 rounded px-1 py-0.5 hover:bg-surface-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium" title={field.description ?? field.expression}>
                    {field.name}
                  </span>
                  <button
                    type="button"
                    className={small}
                    onClick={() => setEditing({ original: field.name })}
                    disabled={editing !== null}
                    aria-label={`Edit calculated field ${field.name}`}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className={`${small} hover:text-danger`}
                    onClick={() => remove(field.name)}
                    disabled={used.length > 0 || editing !== null}
                    title={used.length > 0 ? `Used by ${used.join(", ")}. Remove it from those visuals first.` : `Delete calculated field ${field.name}`}
                    aria-label={`Delete calculated field ${field.name}`}
                  >
                    Delete
                  </button>
                </div>
                <code className="truncate font-mono text-xs text-muted" title={field.expression}>
                  {field.expression}
                </code>
              </li>
            );
          })}
        </ul>
      )}

      {error && (
        <p role="alert" className="break-words text-xs text-danger">
          {error}
        </p>
      )}

      {editing && (
        <CalcFieldForm
          key={editing.original ?? "new"}
          report={report}
          reportPath={reportPath}
          dataset={dataset}
          original={editing.original}
          initial={fields.find((f) => f.name === editing.original) ?? null}
          takenNames={columns.map((c) => c.name).filter((n) => n !== editing.original)}
          onSave={async (field) => {
            const message = await onSave(field, editing.original);
            if (!message) setEditing(null);
            return message;
          }}
          onCancel={() => setEditing(null)}
        />
      )}
      </div>
    </section>
  );
}

function CalcFieldForm({
  report,
  reportPath,
  dataset,
  original,
  initial,
  takenNames,
  onSave,
  onCancel,
}: {
  report: Report;
  reportPath: string | null;
  dataset: string;
  original: string | null;
  initial: CalculatedField | null;
  takenNames: string[];
  onSave: (field: CalculatedField) => Promise<string | null>;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [expression, setExpression] = useState(initial?.expression ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [check, setCheck] = useState<{ status: "idle" | "checking" | "ok" | "error"; message?: string }>({
    status: "idle",
  });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const request = useRef(0);

  const trimmedName = name.trim();
  const trimmedExpression = expression.trim();
  const nameError = calcNameError(trimmedName, takenNames);
  const expressionError = trimmedExpression === "" ? "Enter an expression." : null;

  // Validate after the user pauses typing (and once when the form opens), through the engine.
  // Only the newest request may update the state.
  useEffect(() => {
    if (nameError || expressionError) {
      setCheck({ status: "idle" });
      return;
    }
    const id = ++request.current;
    setCheck({ status: "checking" });
    const candidate = withCalculatedField(report, dataset, { name: trimmedName, expression: trimmedExpression }, original);
    const timer = window.setTimeout(() => {
      api
        .validateCalculatedField(candidate, reportPath, dataset, trimmedName)
        .then(() => {
          if (id === request.current) setCheck({ status: "ok" });
        })
        .catch((err: unknown) => {
          if (id === request.current) setCheck({ status: "error", message: toApiError(err).message });
        });
    }, 400);
    return () => window.clearTimeout(timer);
    // The check depends on what the candidate report would contain, so the report is a dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trimmedName, trimmedExpression, nameError, expressionError, report, reportPath, dataset, original]);

  const canSave = !nameError && !expressionError && check.status === "ok" && !saving;
  const submit = async () => {
    if (!canSave) return;
    setSaving(true);
    setSaveError(null);
    const field: CalculatedField = { name: trimmedName, expression: trimmedExpression };
    if (description.trim()) field.description = description.trim();
    const message = await onSave(field);
    setSaving(false);
    if (message) setSaveError(message);
  };

  const idBase = `calc-${original ?? "new"}`;
  const showExpressionError = expressionError ?? (check.status === "error" ? check.message : null);

  return (
    <form
      aria-label={original ? `Edit calculated field ${original}` : "New calculated field"}
      className="flex flex-col gap-2 rounded-md border border-border bg-surface p-2"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-name`} className="text-xs font-semibold text-muted">
          Name
        </label>
        <input
          id={`${idBase}-name`}
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-invalid={nameError !== null}
          aria-describedby={nameError ? `${idBase}-name-error` : undefined}
          className={control}
          autoComplete="off"
          spellCheck={false}
        />
        {nameError && (
          <p id={`${idBase}-name-error`} className="text-xs text-danger">
            {nameError}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-expression`} className="text-xs font-semibold text-muted">
          Expression
        </label>
        <textarea
          id={`${idBase}-expression`}
          value={expression}
          onChange={(e) => setExpression(e.target.value)}
          rows={3}
          spellCheck={false}
          className={`${control} font-mono text-xs`}
          aria-invalid={showExpressionError !== null}
          aria-describedby={`${idBase}-expression-help ${showExpressionError ? `${idBase}-expression-error` : ""}`.trim()}
          placeholder={'revenue * 0.9  or  CASE WHEN units > 5 THEN "bulk" ELSE "single" END'}
        />
        <p id={`${idBase}-expression-help`} className="text-xs text-muted">
          Columns, constants, arithmetic, comparisons, CASE and a list of allowed functions. No subqueries.
        </p>
        {check.status === "checking" && (
          <p role="status" className="text-xs text-muted">
            Checking expression...
          </p>
        )}
        {showExpressionError && (
          <p id={`${idBase}-expression-error`} role="alert" className="break-words text-xs text-danger">
            {showExpressionError}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-description`} className="text-xs font-semibold text-muted">
          Description (optional)
        </label>
        <input
          id={`${idBase}-description`}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          className={control}
        />
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="submit"
          disabled={!canSave}
          className="min-h-8 rounded-md bg-accent px-3 text-sm font-semibold text-accent-text hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          {saving ? "Saving..." : "Save field"}
        </button>
        <button type="button" onClick={onCancel} className={small}>
          Cancel
        </button>
      </div>
      {saveError && (
        <p role="alert" className="break-words text-xs text-danger">
          {saveError}
        </p>
      )}
    </form>
  );
}

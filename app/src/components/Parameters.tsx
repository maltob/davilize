import { useState, type ReactNode } from "react";
import {
  PARAM_TYPES,
  defaultText,
  paramDeclError,
  paramText,
  paramType,
  parameterUsers,
} from "../model";
import type { ParamType, Parameter, Report } from "../types";

const control =
  "w-full rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent";
const small =
  "rounded-md border border-border px-2 py-0.5 text-xs hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const NAME_RULE = /^[A-Za-z0-9_-]+$/;

function typeLabel(p: Parameter): string {
  return PARAM_TYPES.find((t) => t.value === paramType(p))?.label ?? "Text";
}

/** Declared parameters: list, add, edit and delete. Shown in the data panel. */
export function ParametersSection({
  report,
  onSave,
  onDelete,
}: {
  report: Report;
  onSave: (param: Parameter, original: string | null) => void;
  onDelete: (name: string) => void;
}) {
  const [editing, setEditing] = useState<{ original: string | null } | null>(null);
  const current = editing?.original ? report.parameters.find((p) => p.name === editing.original) ?? null : null;

  return (
    <section aria-labelledby="params-heading" className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <h2 id="params-heading" className="text-xs font-semibold uppercase tracking-wide text-muted">
          Parameters <span className="font-normal">({report.parameters.length})</span>
        </h2>
        <button type="button" className={small} onClick={() => setEditing({ original: null })} disabled={editing !== null}>
          Add parameter
        </button>
      </div>
      {report.parameters.length === 0 && editing === null && (
        <p className="px-2 text-sm text-muted">
          None. A parameter is a value you choose when the report runs, for example a region or a row count.
        </p>
      )}
      {report.parameters.length > 0 && (
        <ul className="flex flex-col gap-1" aria-label="Parameters">
          {report.parameters.map((p) => {
            const used = parameterUsers(report, p.name);
            return (
              <li key={p.name} className="flex flex-col gap-0.5 rounded px-1 py-0.5 hover:bg-surface-2">
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium" title={p.description ?? p.name}>
                    {p.name}
                  </span>
                  <span className="text-xs text-muted">{typeLabel(p)}</span>
                  <button
                    type="button"
                    className="text-xs text-accent underline focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
                    onClick={() => setEditing({ original: p.name })}
                    disabled={editing !== null}
                    aria-label={`Edit parameter ${p.name}`}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className="text-xs text-muted underline hover:text-danger focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:no-underline disabled:opacity-50"
                    onClick={() => {
                      onDelete(p.name);
                    }}
                    disabled={used.length > 0 || editing !== null}
                    title={used.length > 0 ? `Used by ${used.join(", ")}. Remove it from those visuals first.` : undefined}
                    aria-label={`Delete parameter ${p.name}`}
                  >
                    Delete
                  </button>
                </div>
                {used.length > 0 && <span className="text-xs text-muted">Used by {used.join(", ")}</span>}
              </li>
            );
          })}
        </ul>
      )}
      {editing && (
        <ParameterForm
          key={editing.original ?? "new"}
          initial={current}
          takenNames={report.parameters.map((p) => p.name).filter((n) => n !== editing.original)}
          usedBy={current ? parameterUsers(report, current.name) : []}
          onSave={(param) => {
            onSave(param, editing.original);
            setEditing(null);
          }}
          onCancel={() => setEditing(null)}
        />
      )}
    </section>
  );
}

/** Builds a declaration from the form's text. Blank optional fields are left out. */
function buildParameter(form: ParamForm): Parameter {
  const kind = form.type;
  const typed = (text: string): unknown => {
    if (kind === "boolean") return text === "true";
    if (kind === "number" || kind === "integer") return Number(text);
    return text;
  };
  const p: Parameter = { name: form.name.trim(), type: kind };
  if (form.description.trim()) p.description = form.description.trim();
  if (form.defaultText !== "") p.default = typed(form.defaultText);
  const allowed = form.allowedText
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "");
  if (allowed.length > 0) p.allowed = allowed.map(typed);
  if (form.minText !== "") p.min = typed(form.minText);
  if (form.maxText !== "") p.max = typed(form.maxText);
  return p;
}

interface ParamForm {
  name: string;
  type: ParamType;
  defaultText: string;
  description: string;
  allowedText: string;
  minText: string;
  maxText: string;
}

function ParameterForm({
  initial,
  takenNames,
  usedBy,
  onSave,
  onCancel,
}: {
  initial: Parameter | null;
  takenNames: string[];
  usedBy: string[];
  onSave: (param: Parameter) => void;
  onCancel: () => void;
}) {
  const [form, setForm] = useState<ParamForm>(() => ({
    name: initial?.name ?? "",
    type: initial ? paramType(initial) : "string",
    defaultText: initial ? defaultText(initial) : "",
    description: initial?.description ?? "",
    allowedText: (initial?.allowed ?? []).map((a) => String(a)).join(", "),
    minText: initial?.min === undefined ? "" : String(initial.min),
    maxText: initial?.max === undefined ? "" : String(initial.max),
  }));
  const set = <K extends keyof ParamForm>(key: K, value: ParamForm[K]) => setForm((f) => ({ ...f, [key]: value }));
  const id = `param-form-${initial?.name ?? "new"}`;
  const ranged = form.type === "number" || form.type === "integer" || form.type === "date";
  const numeric = form.type === "number" || form.type === "integer";

  const nameError =
    form.name.trim() === ""
      ? "Enter a name."
      : !NAME_RULE.test(form.name.trim())
        ? "Use only letters, digits, _ or -."
        : takenNames.includes(form.name.trim())
          ? `A parameter named "${form.name.trim()}" already exists.`
          : null;
  const allowedBad =
    numeric && form.allowedText.split(",").some((s) => s.trim() !== "" && Number.isNaN(Number(s.trim())))
      ? "Allowed values must be numbers."
      : null;
  const param = buildParameter(form);
  const declError = nameError ?? allowedBad ?? paramDeclError(param);
  const canSave = declError === null;

  return (
    <form
      aria-label={initial ? `Edit parameter ${initial.name}` : "New parameter"}
      className="flex flex-col gap-2 rounded-md border border-border bg-surface p-2"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSave) onSave(param);
      }}
    >
      <div className="flex flex-col gap-1">
        <label htmlFor={`${id}-name`} className="text-xs font-semibold text-muted">
          Name
        </label>
        <input
          id={`${id}-name`}
          value={form.name}
          onChange={(e) => set("name", e.target.value)}
          disabled={usedBy.length > 0}
          aria-invalid={nameError !== null}
          aria-describedby={`${id}-name-help`}
          className={control}
          autoComplete="off"
          spellCheck={false}
        />
        <p id={`${id}-name-help`} className={`text-xs ${nameError ? "text-danger" : "text-muted"}`}>
          {nameError ?? (usedBy.length > 0 ? `Used by ${usedBy.join(", ")}, so it cannot be renamed.` : "Letters, digits, _ or -.")}
        </p>
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-type`} className="text-xs font-semibold text-muted">
            Type
          </label>
          <select id={`${id}-type`} value={form.type} onChange={(e) => set("type", e.target.value as ParamType)} className={control}>
            {PARAM_TYPES.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-default`} className="text-xs font-semibold text-muted">
            Default
          </label>
          <DefaultInput id={`${id}-default`} type={form.type} value={form.defaultText} onChange={(v) => set("defaultText", v)} />
        </div>
      </div>

      {ranged && (
        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-min`} className="text-xs font-semibold text-muted">
              Minimum
            </label>
            <input
              id={`${id}-min`}
              type={form.type === "date" ? "date" : "number"}
              step={form.type === "integer" ? 1 : "any"}
              value={form.minText}
              onChange={(e) => set("minText", e.target.value)}
              className={control}
            />
          </div>
          <div className="flex flex-col gap-1">
            <label htmlFor={`${id}-max`} className="text-xs font-semibold text-muted">
              Maximum
            </label>
            <input
              id={`${id}-max`}
              type={form.type === "date" ? "date" : "number"}
              step={form.type === "integer" ? 1 : "any"}
              value={form.maxText}
              onChange={(e) => set("maxText", e.target.value)}
              className={control}
            />
          </div>
        </div>
      )}

      {form.type !== "boolean" && (
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-allowed`} className="text-xs font-semibold text-muted">
            Allowed values (optional)
          </label>
          <input
            id={`${id}-allowed`}
            value={form.allowedText}
            onChange={(e) => set("allowedText", e.target.value)}
            placeholder="comma separated"
            aria-invalid={allowedBad !== null}
            aria-describedby={allowedBad ? `${id}-allowed-error` : undefined}
            className={control}
          />
          {allowedBad && (
            <p id={`${id}-allowed-error`} className="text-xs text-danger">
              {allowedBad}
            </p>
          )}
        </div>
      )}

      <div className="flex flex-col gap-1">
        <label htmlFor={`${id}-description`} className="text-xs font-semibold text-muted">
          Description (optional)
        </label>
        <input id={`${id}-description`} value={form.description} onChange={(e) => set("description", e.target.value)} className={control} />
      </div>

      {declError && !nameError && !allowedBad && (
        <p role="alert" className="text-xs text-danger">
          {declError}
        </p>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="submit"
          disabled={!canSave}
          className="rounded-md bg-accent px-3 py-1 text-sm font-semibold text-accent-text hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          Save parameter
        </button>
        <button type="button" onClick={onCancel} className={small}>
          Cancel
        </button>
      </div>
    </form>
  );
}

/** Default value input for the declared type. Blank means the parameter is required. */
function DefaultInput({
  id,
  type,
  value,
  onChange,
}: {
  id: string;
  type: ParamType;
  value: string;
  onChange: (value: string) => void;
}) {
  if (type === "boolean") {
    return (
      <select id={id} value={value} onChange={(e) => onChange(e.target.value)} className={control}>
        <option value="">No default (required)</option>
        <option value="true">Yes</option>
        <option value="false">No</option>
      </select>
    );
  }
  return (
    <input
      id={id}
      type={type === "date" ? "date" : type === "number" || type === "integer" ? "number" : "text"}
      step={type === "integer" ? 1 : type === "number" ? "any" : undefined}
      value={value}
      placeholder={value === "" ? "No default (required)" : undefined}
      onChange={(e) => onChange(e.target.value)}
      className={control}
    />
  );
}

/**
 * The values the user has entered for the report's parameters. Changing one
 * re-runs the visuals and the export. Errors are shown beside each input.
 */
export function ParameterValuesBar({
  parameters,
  values,
  issues,
  onChange,
  onReset,
}: {
  parameters: Parameter[];
  values: Record<string, string>;
  issues: Record<string, string>;
  onChange: (name: string, text: string) => void;
  onReset: () => void;
}) {
  return (
    <section
      aria-labelledby="param-values-heading"
      className="flex flex-wrap items-end gap-x-4 gap-y-2 border-b border-border bg-surface px-4 py-2"
    >
      <h2 id="param-values-heading" className="w-full text-xs font-semibold uppercase tracking-wide text-muted md:w-auto md:self-center">
        Parameter values
      </h2>
      {parameters.map((p) => (
        <ParamInput
          key={p.name}
          param={p}
          text={paramText(p, values)}
          error={issues[p.name] ?? null}
          onChange={(text) => onChange(p.name, text)}
        />
      ))}
      <button
        type="button"
        onClick={onReset}
        disabled={Object.keys(values).length === 0}
        className="rounded-md border border-border px-2 py-1 text-xs hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
      >
        Reset to defaults
      </button>
    </section>
  );
}

function ParamInput({
  param,
  text,
  error,
  onChange,
}: {
  param: Parameter;
  text: string;
  error: string | null;
  onChange: (text: string) => void;
}) {
  const id = `param-value-${param.name}`;
  const errorId = `${id}-error`;
  const hintId = `${id}-hint`;
  const describedBy = [param.description ? hintId : null, error ? errorId : null].filter(Boolean).join(" ") || undefined;
  const kind = paramType(param);
  const invalidClass = error ? "border-danger" : "";
  const field = `${control} ${invalidClass} w-auto min-w-40`;

  let input: ReactNode;
  if (kind === "boolean") {
    input = (
      <label className="flex items-center gap-2 text-sm">
        <input
          id={id}
          type="checkbox"
          checked={text === "true"}
          onChange={(e) => onChange(e.target.checked ? "true" : "false")}
          aria-invalid={error !== null}
          aria-describedby={describedBy}
          className="h-4 w-4 accent-accent focus-visible:outline-2 focus-visible:outline-accent"
        />
        <span>{text === "true" ? "Yes" : text === "false" ? "No" : "Not set"}</span>
      </label>
    );
  } else if (param.allowed && param.allowed.length > 0) {
    input = (
      <select
        id={id}
        value={text}
        onChange={(e) => onChange(e.target.value)}
        aria-invalid={error !== null}
        aria-describedby={describedBy}
        className={field}
      >
        {!param.allowed.some((a) => String(a) === text) && <option value="">Choose...</option>}
        {param.allowed.map((a) => (
          <option key={String(a)} value={String(a)}>
            {String(a)}
          </option>
        ))}
      </select>
    );
  } else {
    const inputType = kind === "date" ? "date" : kind === "number" || kind === "integer" ? "number" : "text";
    input = (
      <input
        id={id}
        type={inputType}
        step={kind === "integer" ? 1 : kind === "number" ? "any" : undefined}
        min={param.min !== undefined ? String(param.min) : undefined}
        max={param.max !== undefined ? String(param.max) : undefined}
        value={text}
        onChange={(e) => onChange(e.target.value)}
        aria-invalid={error !== null}
        aria-describedby={describedBy}
        className={field}
        placeholder={defaultText(param) === "" ? "required" : undefined}
      />
    );
  }

  return (
    <div className="flex min-w-0 flex-col gap-1">
      {kind === "boolean" ? (
        <span className="text-xs font-medium">{param.name}</span>
      ) : (
        <label htmlFor={id} className="text-xs font-medium">
          {param.name}
        </label>
      )}
      {input}
      {param.description && (
        <span id={hintId} className="max-w-56 text-xs text-muted">
          {param.description}
        </span>
      )}
      {error && (
        <span id={errorId} className="max-w-56 text-xs text-danger">
          {error}
        </span>
      )}
    </div>
  );
}

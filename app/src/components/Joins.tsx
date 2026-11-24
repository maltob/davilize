import { useEffect, useRef, useState } from "react";
import { api, toApiError } from "../api";
import {
  JOIN_KINDS,
  buildJoinedDataset,
  datasetDependents,
  describeJoin,
  joinKeysError,
  joinKindOf,
  joinNameError,
  joinableRelations,
} from "../model";
import type { ColumnView, Dataset, JoinKey, JoinKind, Report } from "../types";
import { SectionToggle } from "./icons";

const control =
  "min-h-7 w-full rounded-md border border-border bg-surface px-2 py-1 text-sm text-text focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-60";
const small =
  "min-h-7 whitespace-nowrap rounded-md border border-border bg-surface px-2 text-xs font-medium hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent";
const danger = "border-danger text-danger";

interface Props {
  report: Report;
  schemas: Record<string, ColumnView[]>;
  /** Resolves to an error message, or null when the dataset was saved. */
  onSave: (dataset: Dataset, original: string | null) => Promise<string | null>;
  /** Resolves to an error message, or null when the dataset was removed. */
  onDelete: (name: string) => Promise<string | null>;
  /** The form was closed without saving. The engine may hold the checked version, so the saved one is restored. */
  onCancelEdit: (original: string) => void;
}

/** A key row in the form. `id` keeps React keys stable while rows are added and removed. */
interface KeyDraft {
  id: number;
  left: string;
  right: string;
}

/** Joined datasets of the report: list, add, edit and delete. */
export function JoinsSection({ report, schemas, onSave, onDelete, onCancelEdit }: Props) {
  const joined = report.datasets.filter((d) => (d.joins ?? []).length > 0);
  // `null` means the form is closed; `original` is the name being edited, or null for a new dataset.
  const [editing, setEditing] = useState<{ original: string | null } | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(true);

  const remove = async (name: string) => {
    setConfirming(null);
    setError(null);
    const message = await onDelete(name);
    if (message) setError(message);
  };

  // Adding a dataset always shows the section, even when it was collapsed.
  const startNew = () => {
    setOpen(true);
    setError(null);
    setEditing({ original: null });
  };

  return (
    <section aria-labelledby="joins-heading" className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <SectionToggle
          title="Joined datasets"
          count={joined.length}
          open={open}
          controls="joins-section"
          headingId="joins-heading"
          onToggle={() => setOpen((value) => !value)}
        />
        <button type="button" className={small} onClick={startNew} disabled={editing !== null}>
          Add joined dataset
        </button>
      </div>

      <div id="joins-section" hidden={!open} className="space-y-1">
        {joined.length === 0 && editing === null && (
          <p className="px-2 text-xs text-muted">Combine two datasets on matching columns.</p>
        )}

        {joined.length > 0 && (
          <ul className="flex flex-col gap-1" aria-label="Joined datasets list">
            {joined.map((dataset) => {
              const used = datasetDependents(report, dataset.name);
              const text = describeJoin(dataset);
              const locked = used.length > 0;
              return (
                <li key={dataset.name} className="flex flex-col gap-0.5 rounded px-1 py-0.5 hover:bg-surface-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-medium" title={dataset.name}>
                      {dataset.name}
                    </span>
                    {confirming === dataset.name ? (
                      <span role="group" aria-label={`Confirm delete of ${dataset.name}`} className="flex flex-wrap items-center gap-1 text-xs">
                        <span>Delete this dataset? You can undo it.</span>
                        <button
                          type="button"
                          onClick={() => void remove(dataset.name)}
                          aria-label={`Confirm delete of joined dataset ${dataset.name}`}
                          className={`${small} ${danger}`}
                        >
                          Confirm delete
                        </button>
                        <button type="button" onClick={() => setConfirming(null)} className={small}>
                          Cancel
                        </button>
                      </span>
                    ) : (
                      <>
                        <button
                          type="button"
                          className={small}
                          onClick={() => {
                            setError(null);
                            setEditing({ original: dataset.name });
                          }}
                          disabled={editing !== null}
                          aria-label={`Edit joined dataset ${dataset.name}`}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          className={`${small} hover:text-danger`}
                          onClick={() => {
                            setError(null);
                            setConfirming(dataset.name);
                          }}
                          disabled={locked || editing !== null}
                          title={
                            locked
                              ? `Used by ${used.join(", ")}. Remove it from those first.`
                              : `Delete joined dataset ${dataset.name}`
                          }
                          aria-label={`Delete joined dataset ${dataset.name}`}
                        >
                          Delete
                        </button>
                      </>
                    )}
                  </div>
                  <span className="truncate text-xs text-muted" title={text}>
                    {text}
                  </span>
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
          <JoinForm
            key={editing.original ?? "new"}
            report={report}
            schemas={schemas}
            original={editing.original}
            initial={report.datasets.find((d) => d.name === editing.original && (d.joins ?? []).length > 0) ?? null}
            onSave={async (dataset) => {
              const message = await onSave(dataset, editing.original);
              if (!message) setEditing(null);
              return message;
            }}
            onCancel={() => {
              if (editing.original !== null) onCancelEdit(editing.original);
              setEditing(null);
            }}
          />
        )}
      </div>
    </section>
  );
}

function JoinForm({
  report,
  schemas,
  original,
  initial,
  onSave,
  onCancel,
}: {
  report: Report;
  schemas: Record<string, ColumnView[]>;
  original: string | null;
  initial: Dataset | null;
  onSave: (dataset: Dataset) => Promise<string | null>;
  onCancel: () => void;
}) {
  const firstJoin = initial?.joins?.[0];
  const [name, setName] = useState(initial?.name ?? "");
  const [left, setLeft] = useState(initial?.source ?? "");
  const [right, setRight] = useState(firstJoin?.with ?? "");
  const [kind, setKind] = useState<JoinKind>(firstJoin ? joinKindOf(firstJoin) : "inner");
  const nextKey = useRef(0);
  const newKey = (key: JoinKey): KeyDraft => ({ id: nextKey.current++, left: key.left, right: key.right });
  const [keys, setKeys] = useState<KeyDraft[]>(() => {
    return firstJoin?.on.length ? firstJoin.on.map(newKey) : [newKey({ left: "", right: "" })];
  });
  const [check, setCheck] = useState<{ status: "idle" | "checking" | "ok" | "error"; columns?: number; message?: string }>({
    status: "idle",
  });
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const request = useRef(0);

  // The datasets a join may use: those defined before this one (all of them for a new dataset).
  const relations = joinableRelations(report, schemas, original ?? undefined);
  const relationNames = relations.map((r) => r.name);
  // Keep a current value that is not offered (for example a source name written by hand) so it is not lost.
  const leftOptions = [...relationNames, ...(left && !relationNames.includes(left) ? [left] : [])];
  const rightOptions = [...relationNames, ...(right && !relationNames.includes(right) ? [right] : [])];
  const leftColumns = schemas[left] ?? [];
  const rightColumns = schemas[right] ?? [];

  const takenNames = [...report.sources.map((s) => s.name), ...report.datasets.map((d) => d.name)].filter(
    (n) => n !== original,
  );
  const trimmedName = name.trim();
  const nameLocked = original !== null && datasetDependents(report, original).length > 0;
  const nameError = nameLocked ? null : joinNameError(trimmedName, takenNames);
  const setupError =
    relations.length >= 2
      ? null
      : original === null
        ? "Add another data file first. A join needs two datasets."
        : "Only datasets defined before this one can be joined, and fewer than two are.";
  const leftError = left === "" ? "Choose the left dataset." : null;
  const rightError = right === "" ? "Choose the right dataset." : left !== "" && right === left ? "Choose two different datasets." : null;
  const keyList: JoinKey[] = keys.map((k) => ({ left: k.left, right: k.right }));
  const keysError = joinKeysError(keyList);

  const blocker = setupError ?? nameError ?? leftError ?? rightError ?? keysError;
  const candidate = blocker ? null : buildJoinedDataset(trimmedName, left, right, kind, keyList, initial);
  const candidateKey = candidate ? JSON.stringify(candidate) : "";

  // Check through the engine after the user pauses (and once when the form opens). Only the newest request may update the state.
  useEffect(() => {
    if (!candidate) {
      setCheck({ status: "idle" });
      return;
    }
    const id = ++request.current;
    setCheck({ status: "checking" });
    const timer = window.setTimeout(() => {
      api
        .registerDataset(candidate)
        .then((columns) => {
          if (id === request.current) setCheck({ status: "ok", columns: columns.length });
        })
        .catch((err: unknown) => {
          if (id === request.current) setCheck({ status: "error", message: toApiError(err).message });
        });
    }, 400);
    return () => window.clearTimeout(timer);
    // The check depends on the candidate's content only, so its JSON is the dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [candidateKey]);

  const canSave = candidate !== null && check.status === "ok" && !saving;
  const submit = async () => {
    if (!canSave || !candidate) return;
    setSaving(true);
    setSaveError(null);
    const message = await onSave(candidate);
    setSaving(false);
    if (message) setSaveError(message);
  };

  const idBase = `join-${original ?? "new"}`;
  const columnsLabel = (name: string, side: "left" | "right") => (name === "" ? `the ${side} dataset` : name);

  const updateKey = (id: number, patch: Partial<KeyDraft>) =>
    setKeys((list) => list.map((k) => (k.id === id ? { ...k, ...patch } : k)));

  return (
    <form
      aria-label={original ? `Edit joined dataset ${original}` : "New joined dataset"}
      className="flex flex-col gap-2 rounded-md border border-border bg-surface p-2"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {setupError && (
        <p role="alert" className="break-words text-xs text-danger">
          {setupError}
        </p>
      )}

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-name`} className="text-xs font-semibold text-muted">
          Name
        </label>
        <input
          id={`${idBase}-name`}
          value={name}
          onChange={(e) => setName(e.target.value)}
          disabled={nameLocked}
          aria-invalid={nameError !== null}
          aria-describedby={`${idBase}-name-hint`}
          className={control}
          autoComplete="off"
          spellCheck={false}
        />
        <p id={`${idBase}-name-hint`} className={nameError ? "text-xs text-danger" : "text-xs text-muted"}>
          {nameLocked
            ? `The name is locked while ${datasetDependents(report, original ?? "").join(", ")} uses it.`
            : nameError ?? "Letters, digits, _ or -."}
        </p>
      </div>

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-left`} className="text-xs font-semibold text-muted">
          Left dataset
        </label>
        <select
          id={`${idBase}-left`}
          value={left}
          onChange={(e) => {
            const next = e.target.value;
            setLeft(next);
            // Key columns that the new left dataset does not have are cleared, not kept as stale values.
            const columns = schemas[next]?.map((c) => c.name) ?? [];
            setKeys((list) => list.map((k) => ({ ...k, left: columns.includes(k.left) ? k.left : "" })));
          }}
          className={control}
        >
          <option value="">Choose a dataset</option>
          {leftOptions.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </div>

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-kind`} className="text-xs font-semibold text-muted">
          Join type
        </label>
        <select id={`${idBase}-kind`} value={kind} onChange={(e) => setKind(e.target.value as JoinKind)} className={control}>
          {JOIN_KINDS.map((k) => (
            <option key={k.value} value={k.value}>
              {k.label}
            </option>
          ))}
        </select>
      </div>

      <div className="flex flex-col gap-1">
        <label htmlFor={`${idBase}-right`} className="text-xs font-semibold text-muted">
          Right dataset
        </label>
        <select
          id={`${idBase}-right`}
          value={right}
          onChange={(e) => {
            const next = e.target.value;
            setRight(next);
            const columns = schemas[next]?.map((c) => c.name) ?? [];
            setKeys((list) => list.map((k) => ({ ...k, right: columns.includes(k.right) ? k.right : "" })));
          }}
          className={control}
          aria-invalid={rightError !== null && right !== ""}
          aria-describedby={rightError ? `${idBase}-right-error` : undefined}
        >
          <option value="">Choose a dataset</option>
          {rightOptions.map((n) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
        {rightError && right !== "" && (
          <p id={`${idBase}-right-error`} role="alert" className="text-xs text-danger">
            {rightError}
          </p>
        )}
      </div>

      <div role="group" aria-labelledby={`${idBase}-keys`} className="flex flex-col gap-1">
        <p id={`${idBase}-keys`} className="text-xs font-semibold text-muted">
          Matching columns (all must match)
        </p>
        <ul className="flex flex-col gap-1" aria-label="Join keys">
          {keys.map((key, index) => (
            <li key={key.id} className="flex flex-wrap items-center gap-1">
              <select
                aria-label={`Key ${index + 1}: column of ${columnsLabel(left, "left")}`}
                value={key.left}
                onChange={(e) => updateKey(key.id, { left: e.target.value })}
                className={`${control} min-w-0 flex-1`}
              >
                <option value="">Choose column</option>
                {leftColumns.map((c) => (
                  <option key={c.name} value={c.name}>
                    {c.name}
                  </option>
                ))}
              </select>
              <span aria-hidden="true" className="text-xs font-semibold text-muted">
                =
              </span>
              <select
                aria-label={`Key ${index + 1}: column of ${columnsLabel(right, "right")}`}
                value={key.right}
                onChange={(e) => updateKey(key.id, { right: e.target.value })}
                className={`${control} min-w-0 flex-1`}
              >
                <option value="">Choose column</option>
                {rightColumns.map((c) => (
                  <option key={c.name} value={c.name}>
                    {c.name}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className={small}
                disabled={keys.length <= 1}
                title={keys.length <= 1 ? "A join needs at least one key." : `Remove key ${index + 1}`}
                aria-label={`Remove key ${index + 1}`}
                onClick={() => setKeys((list) => list.filter((k) => k.id !== key.id))}
              >
                Remove key
              </button>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className={small}
            onClick={() => setKeys((list) => [...list, newKey({ left: "", right: "" })])}
          >
            Add key
          </button>
          {keysError && <span className="text-xs text-muted">{keysError}</span>}
        </div>
      </div>

      <div className="flex flex-col gap-1">
        {check.status === "checking" && (
          <p role="status" className="text-xs text-muted">
            Checking...
          </p>
        )}
        {check.status === "ok" && (
          <p role="status" className="text-xs text-muted">
            {check.columns} {check.columns === 1 ? "column" : "columns"}
          </p>
        )}
        {check.status === "error" && (
          <p role="alert" className="break-words text-xs text-danger">
            {check.message}
          </p>
        )}
        {check.status === "idle" && blocker && blocker !== setupError && blocker !== nameError && blocker !== rightError && blocker !== keysError && (
          <p role="status" className="text-xs text-muted">
            {blocker}
          </p>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="submit"
          disabled={!canSave}
          className="min-h-8 rounded-md bg-accent px-3 text-sm font-semibold text-accent-text hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-accent"
        >
          {saving ? "Saving..." : "Save joined dataset"}
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

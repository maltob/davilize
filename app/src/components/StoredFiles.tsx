import { useCallback, useEffect, useState } from "react";

import { clearStoredFiles, removeStoredFile, storedFiles } from "../web/files";

interface StoredFilesProps {
  /** Opens a stored report by name. */
  onOpen(name: string): void;
  busy: boolean;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/**
 * Files this browser has kept from earlier visits (web version only): saved reports to reopen
 * and the data files they read. Nothing leaves the browser; "Clear" removes it all.
 */
export function StoredFiles({ onOpen, busy }: StoredFilesProps) {
  const [files, setFiles] = useState<{ name: string; size: number }[] | null>(null);

  const refresh = useCallback(() => {
    storedFiles().then(setFiles, () => setFiles([]));
  }, []);
  useEffect(refresh, [refresh]);

  if (!files || files.length === 0) return null;
  const reports = files.filter((f) => f.name.toLowerCase().endsWith(".dvz"));
  const data = files.filter((f) => !f.name.toLowerCase().endsWith(".dvz"));
  const total = files.reduce((n, f) => n + f.size, 0);
  const button =
    "inline-flex min-h-8 items-center rounded-md border border-border bg-surface px-3 text-sm text-text hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50";

  return (
    <section aria-labelledby="stored-heading" className="flex flex-col gap-2 border-t border-border pt-4">
      <h2 id="stored-heading" className="text-sm font-semibold text-text">
        Kept in this browser
      </h2>
      <p className="text-sm text-muted">
        {reports.length} report{reports.length === 1 ? "" : "s"} and {data.length} data file
        {data.length === 1 ? "" : "s"} ({formatSize(total)}). They stay on this computer, so you do not have to
        choose them again.
      </p>
      {reports.length > 0 && (
        <ul className="flex flex-col gap-1">
          {reports.map((r) => (
            <li key={r.name} className="flex flex-wrap items-center gap-2">
              <button type="button" className={button} disabled={busy} onClick={() => onOpen(r.name)}>
                Open {r.name}
              </button>
              <span className="text-xs text-muted">{formatSize(r.size)}</span>
              <button
                type="button"
                className="text-xs text-muted underline hover:text-text"
                disabled={busy}
                onClick={() => removeStoredFile(r.name).then(refresh)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      <div>
        <button
          type="button"
          className={button}
          disabled={busy}
          onClick={() => clearStoredFiles().then(refresh)}
        >
          Clear everything kept in this browser
        </button>
      </div>
    </section>
  );
}

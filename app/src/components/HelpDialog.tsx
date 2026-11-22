import { useEffect, useRef } from "react";

export const SHORTCUTS: { keys: string; action: string }[] = [
  { keys: "Ctrl+Z", action: "Undo the last change" },
  { keys: "Ctrl+Shift+Z or Ctrl+Y", action: "Redo" },
  { keys: "Ctrl+B", action: "Show or hide the data panel" },
  { keys: "?", action: "Open this help" },
  { keys: "Arrow keys", action: "Move the selected dashboard tile one cell" },
  { keys: "Shift+Arrow keys", action: "Resize the selected dashboard tile" },
  { keys: "Delete", action: "Remove the selected dashboard tile" },
  { keys: "Escape", action: "Cancel a drag in progress" },
];

const GLOSSARY: { term: string; meaning: string }[] = [
  { term: "Data", meaning: "The files you add, such as CSV, Parquet, JSON or Arrow. Each file becomes a dataset." },
  { term: "Field", meaning: "A column in a dataset, listed in the data panel." },
  { term: "Dimension", meaning: "A field that groups or labels results, such as region or month." },
  { term: "Measure", meaning: "A number that is summed, counted or averaged, such as revenue." },
  { term: "Calculated field", meaning: "A new field made from an expression over other fields, such as revenue * 0.9." },
  { term: "Parameter", meaning: "A value you choose when the report runs, such as a region, without editing the chart." },
  { term: "Chart", meaning: "One picture of the data, built from fields on the shelves: bar, line, scatter, table or heatmap." },
  { term: "Report", meaning: "A .dvz file that holds your data links, fields, parameters, charts and dashboards." },
  { term: "Dashboard", meaning: "A grid of charts shown together on one page of a report." },
  { term: "Filter", meaning: "A condition that keeps only some rows, applied to one chart." },
  { term: "Shared filter", meaning: "A filter on a dashboard that applies to every chart on it that has the field." },
];

interface HelpDialogProps {
  open: boolean;
  onClose(): void;
}

/**
 * Keyboard shortcuts, a glossary and where files are saved. A native modal dialog:
 * the browser traps focus, Escape closes it, and focus returns to the element that
 * had it before the dialog opened.
 */
export function HelpDialog({ open, onClose }: HelpDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const opener = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      element.showModal();
    } else if (!open && element.open) {
      element.close();
    }
  }, [open]);

  // Runs whenever the dialog closes, including Escape, which the browser handles itself.
  const handleClose = () => {
    onClose();
    const previous = opener.current;
    opener.current = null;
    if (previous && previous.isConnected) previous.focus();
  };

  return (
    <dialog
      ref={dialog}
      aria-labelledby="help-heading"
      onClose={handleClose}
      onClick={(event) => {
        // A click on the backdrop lands on the dialog itself; clicks inside hit the padded content.
        if (event.target === dialog.current) onClose();
      }}
      className="m-auto max-h-[calc(100dvh-2rem)] w-[min(40rem,calc(100vw-2rem))] overflow-y-auto rounded-lg border border-border bg-surface p-0 text-text shadow-xl backdrop:bg-black/40"
    >
      <div className="flex flex-col gap-6 p-5">
        <header className="flex items-start justify-between gap-4">
          <h2 id="help-heading" className="text-lg font-semibold">
            Help
          </h2>
          <button
            type="button"
            onClick={() => dialog.current?.close()}
            className="min-h-9 rounded-md border border-border px-3 text-sm hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          >
            Close
          </button>
        </header>

        <section aria-labelledby="help-shortcuts" className="flex flex-col gap-2">
          <h3 id="help-shortcuts" className="text-sm font-semibold">
            Keyboard shortcuts
          </h3>
          <table className="w-full border-collapse text-sm">
            <thead className="sr-only">
              <tr>
                <th scope="col">Keys</th>
                <th scope="col">Action</th>
              </tr>
            </thead>
            <tbody>
              {SHORTCUTS.map((shortcut) => (
                <tr key={shortcut.keys} className="border-b border-border last:border-b-0">
                  <th scope="row" className="py-2 pr-4 text-left align-top font-mono text-xs font-medium">
                    {shortcut.keys}
                  </th>
                  <td className="py-2 text-muted">{shortcut.action}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>

        <section aria-labelledby="help-terms" className="flex flex-col gap-2">
          <h3 id="help-terms" className="text-sm font-semibold">
            Terms
          </h3>
          <dl className="flex flex-col gap-2 text-sm">
            {GLOSSARY.map((entry) => (
              <div key={entry.term} className="flex flex-col gap-0.5 sm:flex-row sm:gap-3">
                <dt className="shrink-0 font-medium sm:w-40">{entry.term}</dt>
                <dd className="text-muted">{entry.meaning}</dd>
              </div>
            ))}
          </dl>
        </section>

        <section aria-labelledby="help-files" className="flex flex-col gap-2">
          <h3 id="help-files" className="text-sm font-semibold">
            Where files are saved
          </h3>
          <ul className="list-disc space-y-1 pl-5 text-sm text-muted">
            <li>Reports are saved where you choose, as .dvz files. Data file paths are stored relative to the report's folder.</li>
            <li>Exports (PNG, PDF, HTML, XLSX and CSV) are written to the folder you pick.</li>
            <li>The sample report is written to a davilize-sample folder in your temp folder.</li>
            <li>Your data stays on this computer.</li>
          </ul>
        </section>
      </div>
    </dialog>
  );
}

import { useEffect, useRef } from "react";

interface WelcomeProps {
  onOpenData(): void;
  onOpenReport(): void;
  onOpenSample(): void;
  busy: boolean;
}

const STEPS: { title: string; detail: string }[] = [
  { title: "Add data", detail: "Add a CSV, Parquet, JSON or Arrow file. Each file becomes a dataset you can chart." },
  { title: "Build a chart", detail: "Drag fields onto the shelves, or use the Add to menu, to choose what the chart shows." },
  { title: "Place charts on a dashboard", detail: "Arrange charts in a grid and share filters across them." },
  { title: "Export or publish", detail: "Export charts as PNG, PDF, XLSX or CSV, or save the whole report as one HTML page." },
];

// Only the first time the empty state appears in a session may it take focus.
let firstScreen = true;

/** Empty state shown when no data is loaded. Offers three starting actions and a short checklist. */
export function Welcome({ onOpenData, onOpenReport, onOpenSample, busy }: WelcomeProps) {
  const primary = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const nothingFocused = !document.activeElement || document.activeElement === document.body;
    if (firstScreen && nothingFocused) primary.current?.focus();
    firstScreen = false;
  }, []);

  return (
    <section aria-labelledby="welcome-heading" aria-busy={busy} className="mx-auto flex w-full max-w-2xl flex-col gap-6 px-4 py-8">
      <header className="flex flex-col gap-2">
        <h1 id="welcome-heading" className="text-2xl font-semibold text-text">
          Welcome to Davilize
        </h1>
        <p className="text-base text-muted">
          Davilize turns data files into charts and dashboards that you can export or share.
        </p>
      </header>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-start">
        <div className="flex flex-col gap-1">
          <button
            ref={primary}
            type="button"
            onClick={onOpenSample}
            disabled={busy}
            className="inline-flex min-h-10 items-center justify-center rounded-md bg-accent px-4 text-sm font-semibold text-accent-text hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
          >
            Try the sample report
          </button>
          <span className="px-1 text-xs text-muted">Recommended: a finished report in a few seconds.</span>
        </div>
        <button
          type="button"
          onClick={onOpenData}
          disabled={busy}
          className="inline-flex min-h-9 items-center justify-center rounded-md border border-border bg-surface px-4 text-sm font-medium text-text hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
        >
          Open data files
        </button>
        <button
          type="button"
          onClick={onOpenReport}
          disabled={busy}
          className="inline-flex min-h-9 items-center justify-center rounded-md border border-border bg-surface px-4 text-sm font-medium text-text hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
        >
          Open a report
        </button>
      </div>

      <section aria-labelledby="welcome-steps-heading" className="flex flex-col gap-2 border-t border-border pt-4">
        <h2 id="welcome-steps-heading" className="text-sm font-semibold text-text">
          Four steps
        </h2>
        <ol className="flex flex-col gap-3">
          {STEPS.map((step, index) => (
            <li key={step.title} className="flex gap-3">
              <span
                aria-hidden="true"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-border font-mono text-xs text-muted"
              >
                {index + 1}
              </span>
              <span className="flex flex-col">
                <span className="text-sm font-medium text-text">{step.title}</span>
                <span className="text-sm text-muted">{step.detail}</span>
              </span>
            </li>
          ))}
        </ol>
      </section>

      <p className="text-sm text-muted">Your data stays on this computer. Nothing is uploaded.</p>
    </section>
  );
}

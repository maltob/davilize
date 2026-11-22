import type { ReactNode } from "react";
import type { ChartType, Family } from "../types";

const BADGE: Record<Family, { text: string; label: string }> = {
  number: { text: "#", label: "number" },
  text: { text: "Aa", label: "text" },
  date: { text: "Dt", label: "date" },
  bool: { text: "T/F", label: "true or false" },
  other: { text: "?", label: "other type" },
};

/** Type badge for a field. Decorative: the field's type is also in its tooltip. */
export function TypeBadge({ family, role }: { family: Family; role: "measure" | "dimension" }) {
  const badge = BADGE[family];
  const tone = role === "measure" ? "bg-accent/15 text-accent" : "bg-success/15 text-success";
  return (
    <span
      aria-hidden="true"
      title={`${badge.label} (${role === "measure" ? "measure" : "dimension"})`}
      className={`inline-flex h-5 min-w-7 shrink-0 items-center justify-center rounded px-1 font-mono text-xs font-semibold ${tone}`}
    >
      {badge.text}
    </span>
  );
}

const CHART_PATHS: Record<ChartType, ReactNode> = {
  bar: (
    <>
      <rect x="3" y="11" width="4" height="10" rx="1" />
      <rect x="10" y="6" width="4" height="15" rx="1" />
      <rect x="17" y="3" width="4" height="18" rx="1" />
    </>
  ),
  line: (
    <>
      <path d="M3 18l5-6 4 3 8-10" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="8" cy="12" r="1.6" />
      <circle cx="12" cy="15" r="1.6" />
      <circle cx="20" cy="5" r="1.6" />
    </>
  ),
  scatter: (
    <>
      <circle cx="6" cy="16" r="2" />
      <circle cx="10" cy="9" r="2" />
      <circle cx="14" cy="14" r="2" />
      <circle cx="18" cy="6" r="2" />
      <circle cx="19" cy="18" r="1.5" />
    </>
  ),
  table: (
    <path
      d="M4 4h16a1 1 0 011 1v14a1 1 0 01-1 1H4a1 1 0 01-1-1V5a1 1 0 011-1zm1 5v3h4V9zm6 0v3h8V9zm-6 5v4h4v-4zm6 0v4h8v-4z"
      fillRule="evenodd"
    />
  ),
  heatmap: (
    <>
      <rect x="3" y="3" width="8" height="8" rx="1" opacity=".35" />
      <rect x="13" y="3" width="8" height="8" rx="1" opacity=".7" />
      <rect x="3" y="13" width="8" height="8" rx="1" opacity=".9" />
      <rect x="13" y="13" width="8" height="8" rx="1" opacity=".5" />
    </>
  ),
};

/** Small glyph for a chart type. Decorative: the chart's name is shown beside it. */
export function ChartIcon({ chart }: { chart: ChartType }) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="18" height="18" fill="currentColor" className="mx-auto mb-0.5">
      {CHART_PATHS[chart]}
    </svg>
  );
}

/** Disclosure arrow for collapsible sections. Decorative: the button's text carries the meaning. */
export function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      width="12"
      height="12"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 transition-transform motion-reduce:transition-none ${open ? "rotate-90" : ""}`}
    >
      <path d="M6 4l4 4-4 4" />
    </svg>
  );
}

/**
 * A section heading that shows or hides its content. The button carries
 * `aria-expanded` and controls the panel whose id is `controls`.
 */
export function SectionToggle({
  title,
  count,
  open,
  controls,
  headingId,
  onToggle,
}: {
  title: string;
  count?: number;
  open: boolean;
  controls: string;
  headingId?: string;
  onToggle: () => void;
}) {
  return (
    <h2 id={headingId} className="min-w-0 flex-1">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={controls}
        onClick={onToggle}
        className="flex min-h-7 w-full items-center gap-1.5 rounded px-1 text-left text-xs font-semibold uppercase tracking-wide text-muted hover:text-text focus-visible:outline-2 focus-visible:outline-accent"
      >
        <Chevron open={open} />
        <span className="min-w-0">
          {title}
          {count !== undefined && <span className="font-normal"> ({count})</span>}
        </span>
      </button>
    </h2>
  );
}

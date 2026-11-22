import type { Family } from "../types";

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

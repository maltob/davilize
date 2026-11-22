import { useEffect, useRef, useState } from "react";
import embed, { type Result, type VisualizationSpec } from "vega-embed";

/** Draws a Vega-Lite spec produced by dv-render. The data is embedded in the spec. */
export function ChartView({ spec, label }: { spec: Record<string, unknown>; label: string }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let current = true;
    let result: Result | undefined;
    const dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    setError(null);
    embed(element, spec as VisualizationSpec, { actions: false, renderer: "svg", theme: dark ? "dark" : undefined })
      .then((embedded) => {
        if (current) result = embedded;
        else embedded.finalize();
      })
      .catch((err: unknown) => {
        if (current) setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      current = false;
      result?.finalize();
    };
  }, [spec]);

  return (
    <div className="flex flex-col gap-2">
      <div ref={host} role="img" aria-label={label} className="chart-host overflow-x-auto" />
      {error && <p role="alert" className="text-sm text-danger">The chart could not be drawn: {error}</p>}
    </div>
  );
}

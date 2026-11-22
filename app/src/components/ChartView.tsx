import { useEffect, useMemo, useRef, useState } from "react";
import embed, { type Result, type VisualizationSpec } from "vega-embed";
import {
  TABLE_MAX_ROWS,
  chartSummary,
  chartTable,
  chartThemeConfig,
  readSpec,
  themedSpec,
  type ChartColours,
} from "../chartText";

/** Colours for the chart's text, axes and grid, read from the app's CSS variables (light or dark). */
function readColours(): ChartColours {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return {
    text: token("--color-text", "#17202b"),
    muted: token("--color-muted", "#5a6676"),
    line: token("--color-muted", "#5a6676"),
    grid: token("--color-border", "#d3d9e1"),
  };
}

/**
 * Draws a Vega-Lite spec produced by dv-render, named by a text summary of its data.
 * "Show as table" swaps the chart for an HTML table of the same values. A table chart
 * is always shown as a table.
 */
export function ChartView({ spec, label }: { spec: Record<string, unknown>; label: string }) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [showTable, setShowTable] = useState(false);
  const isTable = useMemo(() => readSpec(spec).kind === "table", [spec]);
  const summary = useMemo(() => chartSummary(spec), [spec]);
  const drawn = !isTable && !showTable;

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let current = true;
    let result: Result | undefined;
    setError(null);
    const themed = themedSpec(spec, chartThemeConfig(readColours())) as VisualizationSpec;
    embed(element, themed, { actions: false, renderer: "svg" })
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
  }, [spec, drawn]);

  return (
    <div className="flex flex-col gap-2">
      {!isTable && (
        <div className="flex justify-end">
          <button
            type="button"
            aria-pressed={showTable}
            onClick={() => setShowTable((on) => !on)}
            className="min-h-7 rounded border border-border bg-surface px-2.5 text-xs font-medium text-text hover:bg-surface-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
          >
            Show as table
          </button>
        </div>
      )}
      {drawn ? (
        <div ref={host} role="img" aria-label={summary} className="chart-host overflow-x-auto" />
      ) : (
        <DataTable spec={spec} caption={label} />
      )}
      {error && <p role="alert" className="text-sm text-danger">The chart could not be drawn: {error}</p>}
    </div>
  );
}

/** The values of a chart as an HTML table: a caption, sticky headers, zebra rows and right-aligned numbers. */
function DataTable({ spec, caption }: { spec: Record<string, unknown>; caption: string }) {
  const table = useMemo(() => chartTable(spec), [spec]);
  return (
    <div className="flex flex-col gap-1">
      <div className="max-h-96 overflow-auto rounded border border-border">
        <table className="w-full border-collapse text-sm text-text">
          <caption className="caption-top px-2 py-1.5 text-left text-sm font-medium">{caption}</caption>
          <thead>
            <tr>
              {table.columns.map((column, index) => (
                <th
                  key={column}
                  scope="col"
                  className={`sticky top-0 z-10 bg-surface-2 px-2 py-1.5 font-semibold ${table.numeric[index] ? "text-right" : "text-left"}`}
                >
                  {column}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row, rowIndex) => (
              <tr key={rowIndex} className="even:bg-surface-2/60">
                {row.map((cell, index) => (
                  <td
                    key={index}
                    className={`border-t border-border px-2 py-1 align-top ${table.numeric[index] ? "text-right tabular-nums" : "text-left"}`}
                  >
                    {cell === null ? "" : String(cell)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {table.truncated && (
        <p className="text-sm text-muted">
          Showing first {TABLE_MAX_ROWS} of {table.totalRows} rows.
        </p>
      )}
    </div>
  );
}

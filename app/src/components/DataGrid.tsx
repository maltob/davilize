import { useMemo, useRef } from "react";
import { flexRender, getCoreRowModel, useReactTable, type ColumnDef } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { QueryTable } from "../types";

type Row = (string | null)[];

const COLUMN_WIDTH = 150;

/** A virtualized grid: only the visible rows are in the DOM. */
export function DataGrid({ table, label }: { table: QueryTable; label: string }) {
  const columns = useMemo<ColumnDef<Row, string | null>[]>(
    () =>
      table.columns.map((column, index) => ({
        id: `c${index}`,
        header: column.name,
        accessorFn: (row: Row) => row[index] ?? null,
        cell: (info) => {
          const value = info.getValue();
          return value === null ? <span className="italic text-muted">NULL</span> : value;
        },
      })),
    [table.columns],
  );
  const instance = useReactTable({ data: table.rows, columns, getCoreRowModel: getCoreRowModel() });
  const rows = instance.getRowModel().rows;
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 28,
    overscan: 12,
  });
  const template = `repeat(${Math.max(columns.length, 1)}, minmax(${COLUMN_WIDTH}px, 1fr))`;

  if (table.columns.length === 0) {
    return <p className="p-3 text-sm text-muted">No columns.</p>;
  }

  return (
    <div
      ref={scrollRef}
      tabIndex={0}
      aria-label={label}
      className="h-full overflow-auto rounded-md border border-border bg-surface text-sm focus-visible:outline-2 focus-visible:outline-accent"
    >
      <div role="table" aria-label={label} aria-rowcount={rows.length + 1} style={{ minWidth: columns.length * COLUMN_WIDTH }}>
        <div role="rowgroup" className="sticky top-0 z-10 bg-surface-2">
          {instance.getHeaderGroups().map((group) => (
            <div key={group.id} role="row" className="grid" style={{ gridTemplateColumns: template }}>
              {table.columns.map((column, index) => (
                <div key={`${column.name}-${index}`} role="columnheader" className="truncate border-b border-border px-2 py-1 font-semibold">
                  {column.name}
                  <span className="ml-1 font-normal text-muted">{column.data_type}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
        <div role="rowgroup" className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index];
            if (!row) return null;
            return (
              <div
                key={row.id}
                role="row"
                aria-rowindex={item.index + 2}
                className="absolute left-0 grid w-full border-b border-border/60"
                style={{
                  gridTemplateColumns: template,
                  height: item.size,
                  transform: `translateY(${item.start}px)`,
                }}
              >
                {row.getVisibleCells().map((cell) => (
                  <div key={cell.id} role="cell" className="truncate px-2 py-1">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

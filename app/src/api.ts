// Thin wrappers over the Tauri commands in app/src-tauri/src/commands.rs.
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AddedDataset,
  ApiError,
  ColumnView,
  Dataset,
  ExportSummary,
  OpenedReport,
  Parameter,
  QueryTable,
  RenderedVisual,
  Report,
  Visual,
} from "./types";

export const DATA_EXTENSIONS = ["csv", "parquet", "json", "jsonl", "ndjson", "arrow", "feather", "ipc"];

export function toApiError(error: unknown): ApiError {
  if (typeof error === "object" && error !== null && "kind" in error && "message" in error) {
    return error as ApiError;
  }
  return { kind: "internal", message: error instanceof Error ? error.message : String(error) };
}

export const api = {
  addDataFiles: (existingNames: string[], paths: string[]) =>
    invoke<AddedDataset[]>("add_data_files", { existingNames, paths }),
  openReport: (path: string) => invoke<OpenedReport>("open_report", { path }),
  saveReport: (path: string, report: Report) => invoke<Report>("save_report", { path, report }),
  previewDataset: (dataset: string, limit: number) =>
    invoke<QueryTable>("preview_dataset", { dataset, limit }),
  profileDataset: (dataset: string) => invoke<QueryTable>("profile_dataset", { dataset }),
  /** `values` maps parameter names to the text the user entered; missing names use defaults. */
  renderVisual: (visual: Visual, parameters: Parameter[], values: Record<string, string>) =>
    invoke<RenderedVisual>("render_visual", { visual, parameters, values }),
  exportReport: (report: Report, format: string, dir: string, values: Record<string, string>) =>
    invoke<ExportSummary>("export_report", { report, format, dir, values }),
  /** Resolves when the field is valid; rejects with an `invalid` error that names the field. */
  validateCalculatedField: (report: Report, reportPath: string | null, dataset: string, field: string) =>
    invoke<void>("validate_calculated_field", { report, reportPath, dataset, field }),
  /** Re-registers a dataset's calculated fields in the open engine; returns its refreshed columns. */
  registerCalculatedFields: (dataset: Dataset) =>
    invoke<ColumnView[]>("register_calculated_fields", { dataset }),
  cancelQuery: () => invoke<void>("cancel_query"),
};

/** Native dialogs. Each returns null when the user cancels. */
export async function pickDataFiles(): Promise<string[] | null> {
  const picked = await open({
    multiple: true,
    filters: [{ name: "Data files", extensions: DATA_EXTENSIONS }],
  });
  return normalize(picked);
}

export async function pickReportToOpen(): Promise<string | null> {
  const picked = await open({ multiple: false, filters: [{ name: "Davilize report", extensions: ["dvz"] }] });
  return normalize(picked)?.[0] ?? null;
}

export async function pickReportToSave(current: string | null): Promise<string | null> {
  return save({
    defaultPath: current ?? "report.dvz",
    filters: [{ name: "Davilize report", extensions: ["dvz"] }],
  });
}

export async function pickFolder(): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false });
  return normalize(picked)?.[0] ?? null;
}

function normalize(picked: string | string[] | null): string[] | null {
  if (picked === null) return null;
  return Array.isArray(picked) ? picked : [picked];
}

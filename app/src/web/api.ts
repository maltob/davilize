// The web page's version of `api` in ../api.ts: same functions, backed by the browser engine.

import type { Dataset, Filter, ImportedReport, Parameter, Report, Visual } from "../types";
import { engine, guard } from "./engine";
import { download } from "./files";
import { unwrap, wasm } from "./wasm";

export const webApi = {
  addDataFiles: (existingNames: string[], paths: string[]) =>
    guard(() => engine.addDataFiles(existingNames, paths)),
  openReport: (path: string) => guard(() => engine.openReport(path)),
  importReport: (path: string): Promise<ImportedReport> => guard(() => engine.importReport(path)),
  saveReport: (path: string, report: Report) =>
    guard(async () => {
      const w = await wasm();
      const text = unwrap<{ text: string }>(w.save_report(JSON.stringify(report))).text;
      download(path, text, "application/json");
      return JSON.parse(text) as Report;
    }),
  previewDataset: (dataset: string, limit: number) => guard(() => engine.previewDataset(dataset, limit)),
  profileDataset: (dataset: string) => guard(() => engine.profileDataset(dataset)),
  renderVisual: (visual: Visual, parameters: Parameter[], values: Record<string, string>) =>
    guard(() => engine.renderVisual(visual, parameters, values)),
  renderDashboardVisual: (visual: Visual, filters: Filter[], parameters: Parameter[], values: Record<string, string>) =>
    guard(() => engine.renderVisual(visual, parameters, values, filters)),
  exportReport: (report: Report, format: string, _dir: string, values: Record<string, string>) =>
    guard(() => engine.exportReport(report, format, values)),
  validateCalculatedField: (report: Report, _reportPath: string | null, dataset: string, field: string) =>
    guard(() => engine.validateCalculatedField(report, dataset, field)),
  registerCalculatedFields: (dataset: Dataset) => guard(() => engine.registerDataset(dataset)),
  registerDataset: (dataset: Dataset) => guard(() => engine.registerDataset(dataset)),
  cancelQuery: () => guard(() => engine.cancel()),
  createSampleReport: () => guard(() => engine.createSample()),
};

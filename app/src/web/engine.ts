// The browser's query engine: DuckDB-Wasm for SQL, dv-wasm (Rust) for everything else.
// It mirrors `dv_engine::Engine` plus app/src-tauri/src/service.rs, so a report behaves the
// same here as in the desktop app.

import embed from "vega-embed";

import type {
  AddedDataset,
  ColumnView,
  Dataset,
  ExportSummary,
  ImportedReport,
  Filter,
  OpenedReport,
  Parameter,
  QueryTable,
  RenderedVisual,
  Report,
  Source,
  Visual,
} from "../types";
import { duck } from "./duck";
import { family, outputStem, relationBase, uniqueRelation } from "./names";
import { assetUrl } from "./platform";
import { allFiles, baseName, download, downloadAll, getFile, hasFile, putFile, rawFilesUnder } from "./files";
import { apiError, unwrap, wasm } from "./wasm";

const quote = (name: string) => `"${name.replaceAll('"', '""')}"`;
const literal = (text: string) => `'${text.replaceAll("'", "''")}'`;

/** Rows a preview returns, as in the desktop app. */
const MAX_PREVIEW_ROWS = 1000;

const FILE_FORMATS: Record<string, "csv" | "parquet" | "json" | "arrow"> = {
  csv: "csv",
  parquet: "parquet",
  pq: "parquet",
  json: "json",
  jsonl: "json",
  ndjson: "json",
  arrow: "arrow",
  feather: "arrow",
  ipc: "arrow",
};

const READERS = { csv: "read_csv_auto", parquet: "read_parquet", json: "read_json_auto" } as const;

function toApiError(error: unknown) {
  if (typeof error === "object" && error !== null && "kind" in error && "message" in error) return error;
  const message = error instanceof Error ? error.message : String(error);
  return apiError(/interrupt|cancel/i.test(message) ? "cancelled" : "query", message);
}

/** Runs `work`, turning any failure into the app's error shape. */
export async function guard<T>(work: () => Promise<T>): Promise<T> {
  try {
    return await work();
  } catch (error) {
    throw toApiError(error);
  }
}

/** Loads an HTML page in a hidden frame, waits for its charts, and opens the print dialog. */
function printPage(html: string): Promise<void> {
  return new Promise((resolve) => {
    const frame = document.createElement("iframe");
    frame.style.cssText = "position:fixed;left:-10000px;top:0;width:1100px;height:800px;border:0";
    frame.srcdoc = html;
    frame.addEventListener("load", () => {
      // Vega loads from a CDN and draws after the page loads.
      setTimeout(() => {
        frame.contentWindow?.focus();
        frame.contentWindow?.print();
        setTimeout(() => frame.remove(), 60_000);
        resolve();
      }, 2500);
    });
    document.body.appendChild(frame);
  });
}

/* eslint-disable @typescript-eslint/no-explicit-any */
class WebEngine {
  /** Lowercase calculated-field names per dataset, to flag their columns. */
  private calculated = new Map<string, Set<string>>();
  /** Views created for the open report, dropped when another report opens. */
  private views: string[] = [];

  private async run(sql: string): Promise<any> {
    const { conn } = await duck();
    return conn.query(sql);
  }

  async describe(select: string): Promise<{ name: string; type: string }[]> {
    const table = await this.run(`DESCRIBE ${select}`);
    return table.toArray().map((row: any) => ({ name: String(row.column_name), type: String(row.column_type) }));
  }

  private async describeOrNull(select: string) {
    try {
      return await this.describe(select);
    } catch {
      return null;
    }
  }

  async columns(relation: string): Promise<ColumnView[]> {
    const calc = this.calculated.get(relation) ?? new Set<string>();
    return (await this.describe(`SELECT * FROM ${quote(relation)}`)).map((c) => {
      const fam = family(c.type);
      return {
        name: c.name,
        data_type: c.type,
        family: fam,
        role: fam === "number" ? ("measure" as const) : ("dimension" as const),
        calculated: calc.has(c.name.toLowerCase()),
      };
    });
  }

  // ---- sources and datasets ----

  async registerSource(source: Source): Promise<void> {
    if (source.type !== "file") {
      throw apiError("source", `Database source '${source.name}' needs the desktop app or the CLI.`);
    }
    const bytes = getFile(source.path);
    if (!bytes) {
      throw apiError("io", `file not found: ${baseName(source.path)}. Choose it again with Add data or Open.`);
    }
    const { db, conn, arrow } = await duck();
    const view = quote(source.name);
    if (source.format === "arrow") {
      const ipc = arrow.tableToIPC(arrow.tableFromIPC(bytes), "stream");
      const table = `${source.name}__arrow`;
      await conn.query(`DROP TABLE IF EXISTS ${quote(table)}`);
      await conn.insertArrowFromIPCStream(ipc, { name: table, create: true });
      await conn.query(`CREATE OR REPLACE VIEW ${view} AS SELECT * FROM ${quote(table)}`);
    } else {
      const name = baseName(source.path);
      // The worker takes ownership of the buffer it is given, so hand it a copy.
      await db.registerFileBuffer(name, bytes.slice());
      await conn.query(
        `CREATE OR REPLACE VIEW ${view} AS SELECT * FROM ${READERS[source.format]}(${literal(name)})`,
      );
    }
    if (!this.views.includes(source.name)) this.views.push(source.name);
  }

  /** Checks a calculated-field expression against the allowlist, using DuckDB's own parser. */
  async checkExpression(name: string, expression: string): Promise<void> {
    const w = await wasm();
    const parse = unwrap<{ sql: string }>(w.calc_parse_query(name, expression)).sql;
    const parsed = await this.run(parse);
    unwrap(w.calc_verify(name, String(parsed.getChildAt(0).get(0))));
  }

  /** The SELECT for a dataset, checked: joins, custom SQL, calculated fields. */
  private async datasetBody(dataset: Dataset): Promise<string> {
    const w = await wasm();
    unwrap(w.check_dataset(JSON.stringify(dataset)));
    const relations: Record<string, string[]> = {};
    const joins = dataset.joins ?? [];
    if (joins.length > 0) {
      for (const relation of new Set([dataset.source, ...joins.map((j) => j.with)])) {
        const described = await this.describeOrNull(`SELECT * FROM ${quote(relation)}`);
        if (described) relations[relation] = described.map((c) => c.name);
      }
    }
    const base = unwrap<{ sql: string }>(w.dataset_base_sql(JSON.stringify(dataset), JSON.stringify(relations))).sql;
    const baseColumns = (await this.describe(base)).map((c) => c.name);
    const fields = dataset.calculated_fields ?? [];
    if (fields.length === 0) return base;

    const { layers } = unwrap<{ layers: { field: string; sql: string }[] }>(
      w.dataset_layers(JSON.stringify(dataset), base, JSON.stringify(baseColumns)),
    );
    for (const [index, field] of fields.entries()) {
      await this.checkExpression(field.name, field.expression);
      try {
        await this.describe(layers[index]!.sql);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        throw apiError(
          "invalid",
          `calculated field '${field.name}' cannot be evaluated on dataset '${dataset.name}': ${reason}`,
        );
      }
    }
    return layers[layers.length - 1]!.sql;
  }

  /** Creates or replaces a dataset's view and returns its columns. A bad edit leaves the old view. */
  async registerDataset(dataset: Dataset): Promise<ColumnView[]> {
    const body = await this.datasetBody(dataset);
    await this.run(`CREATE OR REPLACE VIEW ${quote(dataset.name)} AS ${body}`);
    this.calculated.set(dataset.name, new Set((dataset.calculated_fields ?? []).map((f) => f.name.toLowerCase())));
    if (!this.views.includes(dataset.name)) this.views.push(dataset.name);
    return this.columns(dataset.name);
  }

  /** Checks one calculated field (and the ones before it) without touching the open report. */
  async validateCalculatedField(report: Report, datasetName: string, field: string): Promise<void> {
    const target = report.datasets.find((d) => d.name === datasetName);
    if (!target) throw apiError("invalid", `unknown dataset '${datasetName}'`);
    const fields = target.calculated_fields ?? [];
    const position = fields.findIndex((f) => f.name === field);
    if (position < 0) {
      throw apiError("invalid", `calculated field '${field}' is not defined on dataset '${datasetName}'`);
    }
    await this.datasetBody({ ...target, calculated_fields: fields.slice(0, position + 1) });
  }

  /** Opens a report: drops the previous report's views, then registers sources and datasets. */
  async openReport(path: string): Promise<OpenedReport> {
    const bytes = getFile(path);
    if (!bytes) throw apiError("io", `file not found: ${baseName(path)}`);
    const w = await wasm();
    const report = unwrap<{ report: Report }>(w.open_report(new TextDecoder().decode(bytes))).report;
    const missing = report.sources.flatMap((s) => (s.type === "file" && !getFile(s.path) ? [baseName(s.path)] : []));
    if (missing.length > 0) {
      throw apiError(
        "io",
        `This report reads ${missing.join(", ")}. Choose the data files together with the report.`,
      );
    }
    for (const view of [...this.views].reverse()) {
      await this.run(`DROP VIEW IF EXISTS ${quote(view)} CASCADE`);
    }
    this.views = [];
    this.calculated.clear();
    for (const source of report.sources) await this.registerSource(source);
    const datasets = [];
    for (const dataset of report.datasets) {
      datasets.push({ name: dataset.name, columns: await this.registerDataset(dataset) });
    }
    return { report, path: baseName(path), datasets };
  }

  /** Converts a Tableau or Power BI file (or project folder) to a report and opens it. */
  async importReport(path: string): Promise<ImportedReport> {
    const w = await wasm();
    w.vfs_clear();
    const folder = rawFilesUnder(path);
    const file = getFile(path);
    if (folder.length === 0 && !file) throw apiError("io", `file not found: ${baseName(path)}`);
    // The converters look for the data files next to the file they read.
    for (const [name, bytes] of allFiles()) w.vfs_put(name, bytes);
    for (const [name, bytes] of folder) w.vfs_put(name, bytes);
    const result = unwrap<{ report: Report; warnings: string[]; files: string[] }>(w.interop_import(path, "imported"));
    const warnings = [...result.warnings];
    // Data files packed inside a workbook come out next to the report.
    for (const name of result.files) {
      const bytes = w.vfs_take(name);
      if (bytes) putFile(name, bytes);
    }
    w.vfs_clear();

    // The engine's calculated-field rules, as the desktop app applies them to imported formulas.
    const report = result.report;
    for (const dataset of report.datasets) {
      const kept = [];
      for (const field of dataset.calculated_fields ?? []) {
        try {
          await this.checkExpression(field.name, field.expression);
          kept.push(field);
        } catch (error) {
          const reason = (error as { message?: string }).message ?? String(error);
          warnings.push(`calculated field '${field.name}' was dropped: ${reason}`);
        }
      }
      dataset.calculated_fields = kept;
    }

    const stem = baseName(path).replace(/\.[^.]*$/, "") || "imported";
    let name = `${stem}.dvz`;
    for (let n = 2; hasFile(name); n++) name = `${stem} (${n}).dvz`;
    const text = unwrap<{ text: string }>(w.save_report(JSON.stringify(report))).text;
    putFile(name, new TextEncoder().encode(text));
    try {
      return { opened: await this.openReport(name), warnings };
    } catch (error) {
      download(name, text, "application/json");
      const reason = (error as { message?: string }).message ?? String(error);
      throw apiError(
        "invalid",
        `The file was converted and downloaded as ${name}, but it cannot be opened yet: ${reason}`,
      );
    }
  }

  async addDataFiles(existingNames: string[], paths: string[]): Promise<AddedDataset[]> {
    const taken = new Set(existingNames);
    const added: AddedDataset[] = [];
    for (const path of paths) {
      const file = baseName(path);
      const dot = file.lastIndexOf(".");
      const format = dot >= 0 ? FILE_FORMATS[file.slice(dot + 1).toLowerCase()] : undefined;
      if (!format) throw apiError("invalid", `cannot infer file format from '${file}'`);
      const name = uniqueRelation(relationBase(file.slice(0, dot)), taken);
      taken.add(name);
      taken.add(`${name}_source`);
      const source: Source = { name: `${name}_source`, type: "file", path: file, format };
      await this.registerSource(source);
      const dataset: Dataset = { name, source: source.name, sql: null, joins: [], calculated_fields: [] };
      const columns = await this.registerDataset(dataset);
      added.push({ source, dataset, columns });
    }
    return added;
  }

  // ---- queries ----

  /** A query's result as text cells, cast by DuckDB so dates and decimals read as in the desktop app. */
  private async textTable(select: string): Promise<QueryTable> {
    const columns = await this.describe(select);
    const casted = columns.map((c) => `CAST(${quote(c.name)} AS VARCHAR) AS ${quote(c.name)}`).join(", ");
    const table = await this.run(`SELECT ${casted} FROM (${select})`);
    const rows = table
      .toArray()
      .map((row: any) => columns.map((c) => (row[c.name] === null ? null : String(row[c.name]))));
    return { columns: columns.map((c) => ({ name: c.name, data_type: c.type })), rows };
  }

  previewDataset(dataset: string, limit: number): Promise<QueryTable> {
    const capped = Math.min(Math.max(Math.trunc(limit), 1), MAX_PREVIEW_ROWS);
    return this.textTable(`SELECT * FROM ${quote(dataset)} LIMIT ${capped}`);
  }

  profileDataset(dataset: string): Promise<QueryTable> {
    return this.textTable(`SELECT * FROM (SUMMARIZE ${quote(dataset)})`);
  }

  private async ipc(sql: string): Promise<Uint8Array> {
    const { arrow } = await duck();
    return arrow.tableToIPC(await this.run(sql), "stream");
  }

  async renderVisual(
    visual: Visual,
    parameters: Parameter[],
    values: Record<string, string>,
    filters?: Filter[],
  ): Promise<RenderedVisual> {
    const w = await wasm();
    // Column names are only needed to place a dashboard's shared filters.
    const columns = filters ? (await this.describe(`SELECT * FROM ${quote(visual.dataset)}`)).map((c) => c.name) : [];
    const compiled = unwrap<{ sql: string; visual: Visual }>(
      w.compile_visual(
        JSON.stringify(visual),
        JSON.stringify(parameters),
        JSON.stringify(values),
        filters ? JSON.stringify(filters) : "",
        JSON.stringify(columns),
      ),
    );
    const rendered = unwrap<Omit<RenderedVisual, "sql">>(
      w.prepare_visual(JSON.stringify(compiled.visual), await this.ipc(compiled.sql)),
    );
    return { ...rendered, sql: compiled.sql };
  }

  async cancel(): Promise<void> {
    const { conn } = await duck();
    await conn.cancelSent();
  }

  // ---- export ----

  async exportReport(report: Report, format: string, values: Record<string, string>): Promise<ExportSummary> {
    const w = await wasm();
    const stem = outputStem(report.name);
    const columns: Record<string, string[]> = {};
    for (const dataset of report.datasets) {
      columns[dataset.name] = (await this.describe(`SELECT * FROM ${quote(dataset.name)}`)).map((c) => c.name);
    }
    const plan = unwrap<{ items: { key: string; visual: Visual; sql: string }[] }>(
      w.export_plan(JSON.stringify(report), JSON.stringify(values), JSON.stringify(columns), format === "html"),
    ).items;

    const files: { name: string; bytes: Uint8Array }[] = [];
    const results = async () => {
      const parts: Uint8Array[] = [];
      for (const item of plan) parts.push(await this.ipc(item.sql));
      const joined = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
      let offset = 0;
      for (const p of parts) {
        joined.set(p, offset);
        offset += p.length;
      }
      return { joined, lengths: JSON.stringify(parts.map((p) => p.length)) };
    };

    if (format === "html" || format === "pdf") {
      const { joined, lengths } = await results();
      const html = unwrap<{ html: string }>(
        w.export_html(JSON.stringify(report), JSON.stringify(values), JSON.stringify(columns), joined, lengths),
      ).html;
      if (format === "pdf") {
        // A browser cannot write a PDF file itself: open the print dialog, where "Save as PDF" is offered.
        await printPage(html);
        return { files: [`${stem}.pdf`], warnings: ["Choose Save as PDF in the print dialog."] };
      }
      files.push({ name: `${stem}.html`, bytes: new TextEncoder().encode(html) });
    } else if (format === "xlsx") {
      const { joined, lengths } = await results();
      const visuals = JSON.stringify(plan.map((p) => p.visual));
      const bytes = w.export_xlsx(visuals, joined, lengths);
      if (bytes.length === 0) {
        throw apiError("output", w.xlsx_error(visuals, joined, lengths) || "cannot write the workbook");
      }
      files.push({ name: `${stem}.xlsx`, bytes });
    } else if (format === "csv") {
      const { db } = await duck();
      for (const item of plan) {
        const name = `${item.visual.id}.csv`;
        await this.run(`COPY (${item.sql}) TO ${literal(name)} (HEADER, DELIMITER ',')`);
        const bytes: Uint8Array = await db.copyFileToBuffer(name);
        await db.dropFile(name);
        files.push({ name, bytes });
      }
    } else if (format === "png") {
      for (const item of plan) {
        const rendered = unwrap<Omit<RenderedVisual, "sql">>(
          w.prepare_visual(JSON.stringify(item.visual), await this.ipc(item.sql)),
        );
        const holder = document.createElement("div");
        holder.style.cssText = "position:fixed;left:-10000px;top:0";
        document.body.appendChild(holder);
        try {
          const result = await embed(holder, { ...rendered.spec, background: "white" } as never, {
            actions: false,
            renderer: "canvas",
          });
          const url = await result.view.toImageURL("png", 2);
          const bytes = Uint8Array.from(atob(url.split(",")[1]!), (c) => c.charCodeAt(0));
          files.push({ name: `${item.visual.id}.png`, bytes });
          result.finalize();
        } finally {
          holder.remove();
        }
      }
    } else if (["twb", "twbx", "pbit", "pbip"].includes(format)) {
      w.vfs_clear();
      // Workbooks pack their data, and Power BI models read column types from it.
      for (const source of report.sources) {
        const data = source.type === "file" ? getFile(source.path) : undefined;
        if (source.type === "file" && data) w.vfs_put(baseName(source.path), data);
      }
      const out = format === "pbip" ? `${stem}_pbip` : `${stem}.${format}`;
      const written = unwrap<{ files: string[]; warnings: string[] }>(
        w.interop_export(JSON.stringify(report), format, out),
      );
      for (const name of written.files) {
        const bytes = w.vfs_take(name);
        if (bytes) files.push({ name, bytes });
      }
      w.vfs_clear();
      const names = downloadAll(`${stem}-${format}.zip`, files);
      return { files: names, warnings: written.warnings };
    } else {
      throw apiError(
        "invalid",
        `The ${format} format needs the desktop app or the CLI. The web page writes HTML, Excel, CSV, PNG, PDF (through the print dialog), and Tableau and Power BI template or project files.`,
      );
    }
    const written = downloadAll(`${stem}-${format}.zip`, files);
    return { files: written, warnings: [] };
  }

  /** Puts the bundled sample report and its data in the file store. */
  async createSample(): Promise<string> {
    for (const name of ["sample.csv", "sample.dvz"]) {
      const response = await fetch(assetUrl(`examples/${name}`));
      if (!response.ok) throw apiError("io", `cannot load the sample (${name})`);
      putFile(name, new Uint8Array(await response.arrayBuffer()));
    }
    return "sample.dvz";
  }
}

export const engine = new WebEngine();

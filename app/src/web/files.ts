// A browser has no file system, so files the user picks live in memory under their names.
// A report's relative paths ("sample.csv") are looked up by file name.

import { zipSync } from "fflate";

import { apiError } from "./wasm";

const store = new Map<string, Uint8Array>();

export const baseName = (path: string): string => path.split(/[\/]/).pop() ?? path;

/** Files of a picked project folder, by their path inside it ("Project/Report/definition/report.json"). */
const raw = new Map<string, Uint8Array>();

export function rawFilesUnder(folder: string): [string, Uint8Array][] {
  const prefix = `${folder}/`;
  return [...raw].filter(([name]) => name.startsWith(prefix));
}

export function putFile(name: string, bytes: Uint8Array): void {
  store.set(baseName(name), bytes);
}

export function getFile(path: string): Uint8Array | undefined {
  return store.get(baseName(path));
}

/** Every file the user has picked, by name. */
export function allFiles(): [string, Uint8Array][] {
  return [...store];
}

export function hasFile(path: string): boolean {
  return store.has(baseName(path));
}

/** Shows the browser's file chooser. Resolves to null when the user cancels. */
function chooseFiles(accept: string, multiple: boolean): Promise<File[] | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.multiple = multiple;
    input.style.display = "none";
    document.body.appendChild(input);
    const done = (files: File[] | null) => {
      input.remove();
      resolve(files);
    };
    input.addEventListener("change", () => done(input.files && input.files.length ? [...input.files] : null));
    input.addEventListener("cancel", () => done(null));
    input.click();
  });
}

async function keep(files: File[]): Promise<string[]> {
  const names: string[] = [];
  for (const file of files) {
    putFile(file.name, new Uint8Array(await file.arrayBuffer()));
    names.push(file.name);
  }
  return names;
}

export async function pickDataFilesWeb(extensions: string[]): Promise<string[] | null> {
  const files = await chooseFiles(extensions.map((e) => `.${e}`).join(","), true);
  return files ? keep(files) : null;
}

/** A report and, optionally, the data files it reads. Resolves to the report's name. */
export async function pickReportWeb(): Promise<string | null> {
  const files = await chooseFiles(".dvz,.csv,.parquet,.json,.jsonl,.ndjson,.arrow,.feather,.ipc", true);
  if (!files) return null;
  const names = await keep(files);
  const report = names.find((n) => n.toLowerCase().endsWith(".dvz"));
  if (!report) throw apiError("invalid", "Choose a .dvz report file (and the data files it reads).");
  return report;
}

export function pickReportNameToSaveWeb(current: string | null): string | null {
  const name = window.prompt("Save the report as (a download)", baseName(current ?? "report.dvz"));
  return name && name.trim() ? name.trim() : null;
}

/** Hands a file to the browser's download. */
export function download(name: string, bytes: Uint8Array | string, type = "application/octet-stream"): void {
  const blob = new Blob([bytes as BlobPart], { type });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** One file downloads as itself; several download as one zip. Returns the names written. */
export function downloadAll(zipName: string, files: { name: string; bytes: Uint8Array }[]): string[] {
  if (files.length === 1) {
    download(files[0]!.name, files[0]!.bytes);
    return [files[0]!.name];
  }
  const entries: Record<string, Uint8Array> = {};
  for (const f of files) entries[f.name] = f.bytes;
  download(zipName, zipSync(entries), "application/zip");
  return files.map((f) => f.name);
}

/** A Tableau or Power BI file. Resolves to its name. */
export async function pickImportFileWeb(extensions: string[]): Promise<string | null> {
  const files = await chooseFiles(extensions.map((e) => `.${e}`).join(","), false);
  return files ? (await keep(files))[0]! : null;
}

/** A Power BI Project folder. Resolves to the folder's name. */
export function pickProjectFolderWeb(): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.setAttribute("webkitdirectory", "");
    input.style.display = "none";
    document.body.appendChild(input);
    const done = (value: string | null) => {
      input.remove();
      resolve(value);
    };
    input.addEventListener("change", async () => {
      const files = input.files ? [...input.files] : [];
      if (files.length === 0) return done(null);
      raw.clear();
      for (const file of files) {
        raw.set(file.webkitRelativePath || file.name, new Uint8Array(await file.arrayBuffer()));
      }
      done((files[0]!.webkitRelativePath || files[0]!.name).split("/")[0]!);
    });
    input.addEventListener("cancel", () => done(null));
    input.click();
  });
}

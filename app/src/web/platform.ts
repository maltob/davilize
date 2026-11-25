// The app runs in two places: the desktop shell (Tauri) and a plain web page. The web
// page has no file system or native engine, so it uses `src/web/` instead of Tauri commands.

export const isWeb: boolean = typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);

/** Export formats the web page can write. The rest need the desktop app or the CLI. */
export const WEB_EXPORT_FORMATS = ["html", "xlsx", "csv", "png", "pdf", "twb", "twbx", "pbit", "pbip"];

/** The URL of a file served next to the page (the web build uses relative asset paths). */
export function assetUrl(path: string): string {
  return new URL(`${import.meta.env.BASE_URL}${path}`, document.baseURI).href;
}

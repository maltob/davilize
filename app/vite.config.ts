import { createHash } from "node:crypto";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { type Plugin, defineConfig } from "vite";

/**
 * Web build only: writes `dist/sw.js` from `sw/sw.template.js`, with the list of every file the
 * build produced and a version made from their contents. The service worker caches that list, so
 * a new build is a new version, and an unchanged build keeps its cache.
 */
function serviceWorker(): Plugin {
  let outDir = "dist";
  return {
    name: "davilize-service-worker",
    apply: "build",
    configResolved(config) {
      outDir = resolve(config.root, config.build.outDir);
    },
    closeBundle() {
      const files: string[] = [];
      const walk = (dir: string) => {
        for (const name of readdirSync(dir).sort()) {
          const path = join(dir, name);
          if (statSync(path).isDirectory()) walk(path);
          else files.push(relative(outDir, path).split("\\").join("/"));
        }
      };
      walk(outDir);
      const precache = files.filter((f) => f !== "sw.js");
      const hash = createHash("sha256");
      for (const file of precache) {
        hash.update(file);
        hash.update(readFileSync(join(outDir, file)));
      }
      const template = readFileSync(resolve("sw/sw.template.js"), "utf8");
      const worker = template
        .replace("__VERSION__", hash.digest("hex").slice(0, 12))
        .replace("__PRECACHE__", JSON.stringify(precache));
      writeFileSync(join(outDir, "sw.js"), worker);
    },
  };
}

// Tauri expects a fixed dev port and no screen clearing so its logs stay visible.
export default defineConfig(({ mode }) => ({
  // The web build is served from a sub-path (GitHub Pages), so asset URLs are relative.
  base: mode === "web" ? "./" : "/",
  plugins: [react(), tailwindcss(), ...(mode === "web" ? [serviceWorker()] : [])],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    target: "es2022",
    outDir: "dist",
    chunkSizeWarningLimit: 1600,
    emptyOutDir: true,
  },
}));

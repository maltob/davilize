import { defineConfig } from "@playwright/test";

// Browser tests for the web version. They run against the built site (`bash scripts/build-site.sh`
// writes ./site), served by a small static server. DuckDB-Wasm loads from a CDN, so they need
// network access. Locally, `PW_CHANNEL=chrome` uses an installed Chrome instead of downloading
// Playwright's own browser.
export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  expect: { timeout: 30_000 },
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    channel: process.env.PW_CHANNEL || undefined,
    acceptDownloads: true,
    trace: "retain-on-failure",
  },
  webServer: {
    command: "node e2e/serve.mjs ../site 4173",
    url: "http://127.0.0.1:4173/index.html",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});

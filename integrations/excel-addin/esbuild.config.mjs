// esbuild bundler config for the FinX Excel add-in.
//
// Produces two browser bundles consumed by the manifest's hosted URLs:
//   dist/functions.js  — the Office.js custom-function runtime (FINX.GET, ...)
//   dist/taskpane.js   — the configuration task pane
//
// Run with `npm run build`. The output is plain IIFE-format JS suitable for
// static hosting (the user hosts dist/ behind the https URLs in manifest.xml).
import { build } from "esbuild";

const shared = {
  bundle: true,
  format: "iife",
  platform: "browser",
  target: ["es2020"],
  sourcemap: true,
  minify: process.env.NODE_ENV === "production",
  logLevel: "info",
};

await Promise.all([
  build({
    ...shared,
    entryPoints: ["src/functions/functions.ts"],
    outfile: "dist/functions.js",
  }),
  build({
    ...shared,
    entryPoints: ["src/taskpane/taskpane.ts"],
    outfile: "dist/taskpane.js",
  }),
]);

// eslint-disable-next-line no-console
console.log("FinX Excel add-in bundles written to dist/");

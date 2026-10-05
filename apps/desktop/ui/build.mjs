// Bundles the UI with esbuild into dist/ (no dev server needed; the Rust
// dev server or the Tauri shell serves dist/).
import * as esbuild from "esbuild";
import { cpSync, mkdirSync, rmSync, writeFileSync, readFileSync } from "node:fs";

const watch = process.argv.includes("--watch");
const dev = watch || process.argv.includes("--dev");
rmSync("dist", { recursive: true, force: true });
mkdirSync("dist", { recursive: true });
cpSync("public", "dist", { recursive: true });

const options = {
  entryPoints: { app: "src/main.tsx" },
  bundle: true,
  outdir: "dist",
  format: "esm",
  target: ["es2021", "chrome105", "safari15"],
  jsx: "automatic",
  minify: !dev,
  sourcemap: dev ? "inline" : false,
  legalComments: "linked",
  define: { "process.env.NODE_ENV": dev ? '"development"' : '"production"' },
  loader: { ".svg": "text" },
  logLevel: "info",
};

if (watch) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
  console.log("watching src/ …");
} else {
  const result = await esbuild.build({ ...options, metafile: true });
  writeFileSync("dist/meta.json", JSON.stringify(result.metafile));
  const html = readFileSync("dist/index.html", "utf8");
  if (!html.includes("app.js")) throw new Error("index.html does not load app.js");
}

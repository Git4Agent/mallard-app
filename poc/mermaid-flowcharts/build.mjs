import { build } from "esbuild";
import { readFile, writeFile, mkdir, copyFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, "../..");
const cloud = resolve(app, "../mallardInternal/cloudflare/managed-cloud");
const output = resolve(app, "output/mermaid-flowcharts/web");
await mkdir(output, { recursive: true });
for (const name of ["mermaid-renderer.html", "mermaid-renderer-hashes.json", "mermaid-LICENSE.txt"]) {
  const local = await readFile(resolve(app, "dist", name));
  const remote = await readFile(resolve(cloud, "web/dist", name));
  if (!local.equals(remote)) throw new Error(`Local and Cloud parser asset differs: ${name}`);
  await writeFile(resolve(output, name), local);
}
const widget = await readFile(resolve(here, "child-transcripts.svg"), "utf8");
for (const [surface, name, root, markdown, styles] of [
  ["Local", "desktop", app, "src/components/project-sync/session-detail/SessionDetailEvent.tsx", "src/App.css"],
  ["Cloud", "cloud", cloud, "web/src/components/Markdown.tsx", "web/src/styles.css"],
]) {
  const componentRoot = surface === "Local" ? "src/components" : "web/src/components";
  const entry = `import { createRoot } from "react-dom/client";
import Harness from ${JSON.stringify(resolve(here, "Harness.tsx"))};
import ${surface === "Local" ? "{ SessionDetailMarkdown as Markdown }" : "Markdown"} from ${JSON.stringify(resolve(root, markdown))};
import Widget from ${JSON.stringify(resolve(root, componentRoot, "widgets/ClaudeSvgWidget.tsx"))};
import ${JSON.stringify(resolve(root, styles))};
createRoot(document.getElementById("root")).render(<Harness surface="${surface}" Markdown={Markdown} Widget={Widget} widgetCode={${JSON.stringify(widget)}} />);`;
  const file = resolve(output, `${name}-entry.tsx`);
  await writeFile(file, entry);
  await build({
    absWorkingDir: root, entryPoints: [file], outfile: resolve(output, `${name}.js`),
    bundle: true, format: "esm", platform: "browser", target: "es2020", jsx: "automatic", minify: true,
    external: ["/fonts/*"],
    alias: { react: resolve(root, "node_modules/react"), "react-dom": resolve(root, "node_modules/react-dom"), "react-markdown": resolve(root, "node_modules/react-markdown") },
    loader: { ".woff2": "file", ".svg": "dataurl", ".gif": "dataurl" },
    define: { "process.env.NODE_ENV": '"production"' },
  });
  await writeFile(resolve(output, `${name}.html`), `<!doctype html><html lang="en" data-theme="light"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Mallard ${surface} session diagrams</title><link rel="stylesheet" href="${name}.css"><script type="module" src="${name}.js"></script></head><body><div id="root"></div></body></html>`);
}
await mkdir(resolve(output, "fonts"), { recursive: true });
await copyFile(resolve(cloud, "web/public/fonts/Baloo2-Variable.woff2"), resolve(output, "fonts/Baloo2-Variable.woff2"));
console.log("Built production Local and Cloud session renderers with synthetic fixtures.");

import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const testDirectory = join(projectRoot, "tests", "frontend");
// Optional arguments narrow the run to test files whose names contain one of them.
const filters = process.argv.slice(2);
const entries = (await readdir(testDirectory, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".integration.test.tsx"))
  .filter((entry) => filters.length === 0 || filters.some((filter) => entry.name.includes(filter)))
  .map((entry) => join("tests", "frontend", entry.name))
  .sort();
const outputDirectory = await mkdtemp(join(tmpdir(), "agent-sync-frontend-tests-"));

try {
  const outputs = [];
  for (const entry of entries) {
    const outputFile = join(outputDirectory, `${basename(entry, ".tsx")}.cjs`);
    await build({
      absWorkingDir: projectRoot,
      entryPoints: [entry],
      outfile: outputFile,
      bundle: true,
      platform: "node",
      format: "cjs",
      target: "node20",
      loader: { ".woff": "file", ".woff2": "file", ".ttf": "file" },
      define: { "process.env.NODE_ENV": '"test"' },
      logLevel: "warning",
    });
    outputs.push(outputFile);
  }
  // The suite asserts English output. Node takes `navigator.languages` and its
  // default number and date locale from the system's, so on a Chinese machine
  // the app's System language would be Chinese; pin the run to English.
  const result = spawnSync(process.execPath, ["--test", ...outputs], {
    cwd: projectRoot,
    env: { ...process.env, LANG: "en_US.UTF-8", LC_ALL: "en_US.UTF-8" },
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exitCode = result.status ?? 1;
} finally {
  await rm(outputDirectory, { recursive: true, force: true });
}

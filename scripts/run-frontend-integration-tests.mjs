import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const testDirectory = join(projectRoot, "tests", "frontend");
const entries = (await readdir(testDirectory, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".integration.test.tsx"))
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
      define: { "process.env.NODE_ENV": '"test"' },
      logLevel: "warning",
    });
    outputs.push(outputFile);
  }
  const result = spawnSync(process.execPath, ["--test", ...outputs], {
    cwd: projectRoot,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exitCode = result.status ?? 1;
} finally {
  await rm(outputDirectory, { recursive: true, force: true });
}

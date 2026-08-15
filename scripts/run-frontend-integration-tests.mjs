import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDirectory = await mkdtemp(join(tmpdir(), "agent-sync-frontend-tests-"));
const entries = [
  "tests/frontend/activity-log.integration.test.tsx",
  "tests/frontend/conversation-path-repair.integration.test.tsx",
  "tests/frontend/css-hygiene.integration.test.tsx",
  "tests/frontend/pull-review.integration.test.tsx",
  "tests/frontend/project-activity.integration.test.tsx",
  "tests/frontend/project-files.integration.test.tsx",
  "tests/frontend/project-sync-cutover.integration.test.tsx",
  "tests/frontend/resource-inventory.integration.test.tsx",
  "tests/frontend/session-detail.integration.test.tsx",
  "tests/frontend/storage-catalog.integration.test.tsx",
  "tests/frontend/storage-settings.integration.test.tsx",
  "tests/frontend/thread-variants.integration.test.tsx",
];

try {
  const outputs = [];
  for (const entry of entries) {
    const outputFile = join(outputDirectory, `${entry.split("/").at(-1).replace(/\.tsx$/, "")}.cjs`);
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

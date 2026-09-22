import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { requireValue } from "./drive.mjs";

export const REPOSITORY_ROOT = fileURLToPath(new URL("../../", import.meta.url));

export function buildSelector() {
  const build = spawnSync("cargo", [
    "build", "--offline", "--locked", "-p", "mallard-domain",
    "--example", "feishu_publication_selector", "--message-format=json",
  ], { cwd: REPOSITORY_ROOT, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, timeout: 120_000 });
  requireValue(build.status === 0, "selector_build_failed");
  const artifacts = build.stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line));
  const executable = artifacts.find((entry) => entry.reason === "compiler-artifact"
    && entry.target.name === "feishu_publication_selector" && entry.executable)?.executable;
  requireValue(typeof executable === "string", "selector_artifact_missing");
  return (request) => {
    const result = spawnSync(executable, [], {
      input: JSON.stringify(request), encoding: "utf8", timeout: 10_000, maxBuffer: 32 * 1024 * 1024,
    });
    requireValue(result.status === 0, "selector_failed");
    return JSON.parse(result.stdout);
  };
}

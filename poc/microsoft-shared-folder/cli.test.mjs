import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, symlink, chmod, rm, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { parseConfiguration, readConfiguration } from "./cli.mjs";

const config = { clientId: "11111111-1111-4111-8111-111111111111",
  tenant: "22222222-2222-4222-8222-222222222222",
  sharingUrl: "https://fixture.sharepoint.com/:f:/s/test/PRIVATE_LINK" };

test("configuration selects only the resource tenant and declared shared-folder permissions", () => {
  assert.deepEqual(parseConfiguration(config).scopes, ["User.Read", "Files.ReadWrite.All"]);
  for (const extra of [{ accessToken: "PRIVATE_TOKEN" }, { clientSecret: "PRIVATE_SECRET" }, { scopes: ["Sites.Read.All"] }])
    assert.throws(() => parseConfiguration({ ...config, ...extra }), { code: "invalid_configuration" });
  assert.throws(() => parseConfiguration({ ...config, tenant: "organizations" }), { code: "resource_tenant_required" });
});

test("config reader accepts a private regular file and rejects links, open permissions and oversized files", async () => {
  const dir = await realpath(await mkdtemp(join(tmpdir(), "mallard-ms-poc-")));
  const path = join(dir, "config.json");
  try {
    await writeFile(path, JSON.stringify(config), { mode: 0o600 });
    assert.equal((await readConfiguration(path)).sharingUrl, config.sharingUrl);
    await symlink(path, join(dir, "link.json"));
    await assert.rejects(readConfiguration(join(dir, "link.json")), { code: "configuration_must_be_canonical" });
    if (process.platform !== "win32") {
      await chmod(path, 0o644);
      await assert.rejects(readConfiguration(path), { code: "configuration_must_be_private" });
      await chmod(path, 0o600);
    }
    await writeFile(path, "x".repeat(16 * 1024 + 1));
    await assert.rejects(readConfiguration(path), { code: "invalid_configuration_file" });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test("CLI help and failures never open a browser or echo input paths and values", () => {
  const entry = new URL("./cli.mjs", import.meta.url);
  const run = (args) => spawnSync(process.execPath, [entry.pathname, ...args], { encoding: "utf8" });
  const help = run(["--help"]);
  assert.equal(help.status, 0);
  assert.match(help.stdout, /--write-probe/u);
  for (const args of [["--secret=PRIVATE_SECRET"], ["--config", "/PRIVATE_PATH/does-not-exist"], []]) {
    const result = run(args);
    assert.equal(result.status, 1);
    assert.doesNotMatch(result.stdout + result.stderr, /PRIVATE|stack|Error:/u);
    assert.match(result.stderr, /failureCode/u);
  }
});

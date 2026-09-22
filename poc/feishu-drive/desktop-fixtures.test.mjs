import assert from "node:assert/strict";
import { cp, mkdtemp, open, readFile, rename, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { prepareDesktopFixtures, verifyDesktopDownload } from "./desktop-fixtures.mjs";
import { STREAM } from "./probe.mjs";

test("desktop evidence rejects changed bytes and a renamed winning record", async () => {
  const fixture = await prepareDesktopFixtures();
  const download = await mkdtemp(join(tmpdir(), "feishu-desktop-verifier-"));
  try {
    await cp(fixture.upload, download, { recursive: true });
    const good = await verifyDesktopDownload(fixture.manifest, download);
    assert.equal(good.verdict, "PASS");
    assert.equal(good.matched_files, 8);
    assert.equal(good.record_count, 3);
    assert.equal(good.selected_chain_length, 2);

    // Preserve size but change one byte, as a transport corruption would.
    const file = await open(join(download, "over-20MiB.bin"), "r+");
    try { await file.write(Buffer.from([0]), 0, 1, 1024); } finally { await file.close(); }
    const corrupt = await verifyDesktopDownload(fixture.manifest, download);
    assert.equal(corrupt.verdict, "FAIL");
    assert.deepEqual(corrupt.changed, ["over-20MiB.bin"]);

    const manifest = JSON.parse(await readFile(fixture.manifest, "utf8"));
    const winner = `${STREAM}/${manifest.selected_chain.at(-1)}`;
    await rename(join(download, winner), join(download, "renamed.json"));
    const renamed = await verifyDesktopDownload(fixture.manifest, download);
    assert.equal(renamed.verdict, "FAIL");
    assert.deepEqual(renamed.missing, [winner]);
    assert.deepEqual(renamed.extra, ["renamed.json"]);
    assert.equal(renamed.selection_matches, false);
  } finally {
    await rm(fixture.directory, { recursive: true, force: true });
    await rm(download, { recursive: true, force: true });
  }
});

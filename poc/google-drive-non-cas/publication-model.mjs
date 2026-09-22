// Executable model of the current Rust publication rules, not a product engine.
// Native parity tests compile the actual Rust RecordName and Mainline source.
import { MAX_RECORD_BYTES, PocError, sha256 } from "./drive-store.mjs";

export const PROJECT_RECORDS = ".mallard/v1/repositories/_commits";
const TOKEN = "commit-[a-f0-9]{24}-[a-f0-9]{64}";
const NAME = new RegExp(`^(\\d{10})\\.(${TOKEN})\\.(root|${TOKEN})\\.(\\d{13})\\.json$`, "u");

export function parseName(name) {
  const match = NAME.exec(name);
  if (!match) return null;
  const revision = Number(match[1]);
  const parent = match[3] === "root" ? null : match[3];
  if (revision === 0 || (revision === 1) !== (parent === null) || parent === match[2]) return null;
  return { name, revision, token: match[2], parent, time: Number(match[4]), digest: match[2].slice(-64) };
}

export function makeRecord(parent, id, time, fixture = "synthetic publication") {
  if (!/^[a-f0-9]{24}$/u.test(id) || !Number.isSafeInteger(time) || time < 0 || time > 9_999_999_999_999) {
    throw new PocError("invalid_record_input");
  }
  // The Rust publication transport treats records as opaque, hashed bytes.
  // These fixture bytes are not a BundleCommit or LibraryResourceCommit DTO.
  const bytes = Buffer.from(JSON.stringify({ fixture, id }));
  const revision = parent ? parent.revision + 1 : 1;
  const name = `${String(revision).padStart(10, "0")}.commit-${id}-${sha256(bytes)}.${parent?.token ?? "root"}.${String(time).padStart(13, "0")}.json`;
  const record = parseName(name);
  if (!record) throw new PocError("invalid_record_input");
  return { ...record, bytes };
}

export function selectMainline(names) {
  const records = [...new Set(names)].map(parseName).filter(Boolean);
  const children = new Map();
  for (const record of records) {
    if (!children.has(record.parent)) children.set(record.parent, []);
    children.get(record.parent).push(record);
  }
  const chain = [];
  let parent = null;
  for (;;) {
    const candidates = (children.get(parent) ?? []).filter((r) => r.revision === chain.length + 1);
    candidates.sort((a, b) => a.time - b.time || (a.token < b.token ? -1 : a.token > b.token ? 1 : 0));
    if (candidates.length === 0) return chain;
    chain.push(candidates[0]);
    parent = candidates[0].token;
  }
}

export async function listView(store, stream) {
  const names = (await store.listKeys(stream)).map((key) => key.slice(stream.length + 1))
    .filter((name) => !name.includes("/") && parseName(name));
  return { names, selected: selectMainline(names).at(-1) ?? null };
}

export async function publish(store, stream, record, { afterList, now = Date.now } = {}) {
  if (record.bytes.length > MAX_RECORD_BYTES || sha256(record.bytes) !== record.digest) {
    throw new PocError("invalid_record_bytes");
  }
  // Allocate the immutable file ID before the time-sensitive list/write step.
  const prepared = await store.prepare(`${stream}/${record.name}`, record.bytes);
  let requests = 0;
  let acknowledged = false;
  let sendError;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    let view;
    try { view = await listView(store, stream); }
    catch (error) {
      if (requests > 0) throw new PocError("outcome_unknown");
      throw error;
    }
    const listedAt = now();
    if (acknowledged && !view.names.includes(record.name)) {
      view.names.push(record.name);
      view.selected = selectMainline(view.names).at(-1) ?? null;
    }
    if (view.names.some((name) => parseName(name).token === record.token)) {
      if (view.selected?.name !== record.name) throw new PocError("remote_changed");
      return view;
    }
    if (record.parent === null ? view.names.length !== 0 : view.selected?.token !== record.parent) {
      throw new PocError("remote_changed");
    }
    if (requests === 3) break;
    const age = now() - listedAt;
    if (age < 0 || age > 10_000) continue;
    // Test hook at the same point as Rust's before-record-request hook.
    await afterList?.({ attempt, view });
    requests += 1;
    try { await store.writeOnce(prepared); acknowledged = true; }
    catch (error) { sendError = error; }
  }
  throw sendError ?? new PocError("listing_not_fresh");
}

export async function readRecord(store, stream, record) {
  const bytes = await store.get(`${stream}/${record.name}`, { maxBytes: MAX_RECORD_BYTES });
  if (bytes === null || sha256(bytes) !== record.digest) throw new PocError("record_digest_mismatch");
  return bytes;
}

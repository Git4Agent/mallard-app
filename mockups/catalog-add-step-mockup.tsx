import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { storageIconName, type IconName } from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import { FieldMessage } from "../src/components/notice/Notice";
import "../src/App.css";

/**
 * Mockup: Add Catalog resource as New project's two steps, one row per pass.
 *
 * Status (2026-10-02): the owner picked Home first. Its agent step drew the
 * home as New project draws a folder; `catalog-add-home-mockup.html` redraws
 * it as the project's Agent band, with the project's preselection rule.
 *
 * The owner picked the two-step shape of `catalog-add-mockup.html`'s B and
 * set one rule on it: a pass sets up one Catalog row — Pets v1 on one home,
 * say — never a batch. So every list below is a radio, and Finish makes
 * exactly one row, as Add does today. What changes is that the page reads
 * before it asks: what this computer holds is found and pre-chosen, the
 * home and the storage have defaults with a stated reason, and the button
 * names what it will do.
 *
 *   Kind first   step one picks the kind — the found list, Skills already
 *                chosen — and step two is "Set up Skills": its home, with the
 *                saved profile chosen, then its storage, then Finish. The
 *                sidebar's and landing's Kind doors skip step one. Recommended.
 *   Home first   B as mocked: step one picks the home, step two is "Set up
 *                Default Codex" with one kind chosen among those it holds.
 *
 * Shipped classes and real leaf components; static data standing in for one
 * new backend read (a count per capability root — see the notes). Every panel
 * keeps its own state. Dev server only: `npm run dev`, then
 * /mockups/catalog-add-step-mockup.html
 * (`?stock=some|none&storages=none|one|many&catalog=empty|some&door=any|skill|pet-v1&theme=dark`).
 */

/* ——— data ——————————————————————————————————————————————————————— */

type ProviderKey = "codex" | "claude";
interface Provider { key: ProviderKey; label: string }
const PROVIDERS: Provider[] = [
  { key: "codex", label: "Codex" },
  { key: "claude", label: "Claude" },
];
const provider = (key: ProviderKey): Provider => PROVIDERS.find((candidate) => candidate.key === key)!;

/* The registry this build registers, grouped by shape the way the chooser's
   cards are (`catalogDoors`), in the backend's order. */
interface Shape { key: string; label: string; format: string; noun: [string, string]; icon: IconName; providers: ProviderKey[] }
const SHAPES: Shape[] = [
  { key: "skill", label: "Skills", format: "Agent Skill", noun: ["skill", "skills"], icon: "lightbulb", providers: ["codex", "claude"] },
  { key: "plugin", label: "Plugins", format: "Plugin install intent", noun: ["plugin", "plugins"], icon: "catalog-plugin", providers: ["codex", "claude"] },
  { key: "pet-v1", label: "Pets v1", format: "Codex pet v1", noun: ["pet", "pets"], icon: "catalog-pet-v1", providers: ["codex"] },
  { key: "pet-v2", label: "Pets v2", format: "Codex pet v2", noun: ["pet", "pets"], icon: "catalog-pet-v2", providers: ["codex"] },
];
const shape = (key: string): Shape => SHAPES.find((candidate) => candidate.key === key)!;
const noun = (kind: Shape, count: number) => kind.noun[count === 1 ? 0 : 1];

/** "Codex only", "Codex and Claude" — `catalogProviderList` verbatim. */
function providerList(labels: readonly string[]): string {
  if (labels.length === 1) return `${labels[0]} only`;
  return `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

/* `list_catalog_resource_profiles` registers these itself when the default
   directories exist; nobody typed them. Listed most recently active first. */
interface Home { id: string; provider: ProviderKey; name: string; path: string; lastActive: string }
const HOMES: Home[] = [
  { id: "codex-default", provider: "codex", name: "Default Codex", path: "~/.codex", lastActive: "2 hours ago" },
  { id: "claude-default", provider: "claude", name: "Default Claude", path: "~/.claude", lastActive: "yesterday" },
];
const HOME_DIR = "/Users/you";
const expand = (path: string) => (path === "~" ? HOME_DIR : path.startsWith("~/") ? `${HOME_DIR}/${path.slice(2)}` : path);

/* What the one new read answers, per shape per home: the roots the
   capability would scan under that home and how many entries each holds
   (null: the root does not exist), plus the newest entry's stamp. A readdir
   per root, bounded and no-follow — never an inventory, never a hash. */
interface Found { shape: string; home: string; roots: { path: string; count: number | null }[]; stamp: string | null }
const FOUND: Found[] = [
  { shape: "skill", home: "codex-default", roots: [{ path: "~/.codex/skills", count: 9 }, { path: "~/.agents/skills", count: 3 }], stamp: "2 hours ago" },
  { shape: "plugin", home: "codex-default", roots: [{ path: "~/.codex/plugins", count: 3 }], stamp: "3 days ago" },
  { shape: "pet-v1", home: "codex-default", roots: [{ path: "~/.codex/pets", count: 0 }], stamp: null },
  { shape: "pet-v2", home: "codex-default", roots: [{ path: "~/.codex/pets", count: 2 }], stamp: "last week" },
  { shape: "skill", home: "claude-default", roots: [{ path: "~/.claude/skills", count: 4 }], stamp: "yesterday" },
  { shape: "plugin", home: "claude-default", roots: [{ path: "~/.claude/plugins", count: null }], stamp: null },
];
const foundTotal = (found: Found | undefined) => found?.roots.reduce((sum, root) => sum + (root.count ?? 0), 0) ?? 0;
const rowKey = (shapeKey: string, homeId: string) => `${shapeKey}:${homeId}`;

interface Storage { id: string; name: string; provider: string; providerLabel: string }
const STORAGES: Storage[] = [
  { id: "vault", name: "Vault", provider: "r2", providerLabel: "Cloudflare R2" },
  { id: "cloud", name: "Mallard Cloud", provider: "managed_cloud", providerLabel: "Mallard Cloud · kai" },
  { id: "backup", name: "Backup drive", provider: "local", providerLabel: "Local folder" },
];

/* The canned machine behind a custom path: one folder per provider answers,
   and holds a few skills and nothing else. */
const CUSTOM: Record<ProviderKey, { path: string; skills: number }> = {
  codex: { path: "~/work/.codex", skills: 1 },
  claude: { path: "~/work/.claude", skills: 2 },
};

/* ——— knobs ———————————————————————————————————————————————————————— */

type Stock = "some" | "none";
type StorageStock = "none" | "one" | "many";
type CatalogStock = "empty" | "some";
type Door = "any" | "skill" | "pet-v1";
interface Knobs { stock: Stock; storages: StorageStock; catalog: CatalogStock; door: Door }

function readKnobs(): Knobs {
  const params = new URLSearchParams(location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = params.get(key);
    return allowed.includes(value as T) ? (value as T) : fallback;
  };
  return {
    stock: pick("stock", ["some", "none"], "some"),
    storages: pick("storages", ["none", "one", "many"], "one"),
    catalog: pick("catalog", ["empty", "some"], "empty"),
    door: pick("door", ["any", "skill", "pet-v1"], "any"),
  };
}

/** One machine, as the knobs describe it. */
interface Scenario {
  homes: Home[];
  found: Found[];
  storages: Storage[];
  /** Rows already in the Catalog, as "shape:home". */
  added: Set<string>;
  /** The storage a Catalog row used last, when one did. */
  lastUsed: string | null;
  /** The Kind door the page was opened from, when one was. */
  door: Door;
}

function scenario(knobs: Knobs): Scenario {
  const stocked = knobs.stock === "some";
  return {
    homes: stocked ? HOMES : [],
    found: stocked ? FOUND : [],
    storages: knobs.storages === "none" ? [] : knobs.storages === "one" ? STORAGES.slice(0, 1) : STORAGES,
    added: new Set(knobs.catalog === "some" ? ["skill:codex-default"] : []),
    lastUsed: knobs.catalog === "some" && knobs.storages !== "none" ? "vault" : null,
    door: knobs.door,
  };
}

const foundFor = (scene: Scenario, kind: Shape, home: Home) =>
  scene.found.find((found) => found.shape === kind.key && found.home === home.id);
/** The homes a kind can be set up on: those of the providers it is
 *  registered for, most recently active first. */
const homesFor = (scene: Scenario, kind: Shape) =>
  scene.homes.filter((home) => kind.providers.includes(home.provider));
const isAdded = (scene: Scenario, kind: Shape, home: Home) => scene.added.has(rowKey(kind.key, home.id));
/** Every home this kind could go on already has its row. */
const fullyAdded = (scene: Scenario, kind: Shape) => {
  const homes = homesFor(scene, kind);
  return homes.length > 0 && homes.every((home) => isAdded(scene, kind, home));
};
const kindTotal = (scene: Scenario, kind: Shape) =>
  homesFor(scene, kind).reduce((sum, home) => sum + foundTotal(foundFor(scene, kind, home)), 0);

/* ——— the defaults ————————————————————————————————————————————————— */

/** Step one's answer before anyone clicks: the first kind in the backend's
 *  order with something on this computer and a home still without its row.
 *  A Kind door is the answer outright. Nothing found: no guess. */
function defaultKind(scene: Scenario): string | null {
  if (scene.door !== "any") return scene.door;
  return SHAPES.find((kind) => kindTotal(scene, kind) > 0 && !fullyAdded(scene, kind))?.key ?? null;
}

/** The home a kind is set up on before anyone clicks: the one holding the
 *  most of it that has no row yet, then the most recently active. With no
 *  home for its providers, the first provider and a path to type. */
function defaultHome(scene: Scenario, kind: Shape): { provider: ProviderKey; home: Home | null } {
  const open = homesFor(scene, kind).filter((home) => !isAdded(scene, kind, home));
  const best = [...open].sort((left, right) => foundTotal(foundFor(scene, kind, right)) - foundTotal(foundFor(scene, kind, left)))[0] ?? null;
  return best ? { provider: best.provider, home: best } : { provider: kind.providers[0]!, home: null };
}

/** The home step one of Home first chooses: the most recently active. */
const defaultHomeFirst = (scene: Scenario) => scene.homes[0]?.id ?? null;

/** The storage the row starts on: the one a Catalog row used last, else the
 *  only one there is, else none — New project's "None yet" when there is
 *  nothing to choose. Never the first in the list. */
function defaultStorage(scene: Scenario): string | null {
  if (scene.lastUsed && scene.storages.some((storage) => storage.id === scene.lastUsed)) return scene.lastUsed;
  return scene.storages.length === 1 ? scene.storages[0]?.id ?? null : null;
}

/** The sentence under the storage row: what the default is, and why. */
function storageHint(storages: Storage[], value: string | null, lastUsed: string | null): string {
  if (storages.length === 0) return "No storage yet. The row stays on this computer; once a storage exists, choose it from the row's storage menu.";
  const chosen = storages.find((storage) => storage.id === value);
  if (!chosen) return "The row stays on this computer until a storage is chosen from its storage menu.";
  const why = chosen.id === lastUsed ? "the storage your Catalog used last" : storages.length === 1 ? "your only storage" : null;
  return `The row compares and syncs with ${chosen.name}${why ? `, ${why}` : ""}. Change it any time from its storage menu.`;
}

/* ——— the custom path ——————————————————————————————————————————————— */

interface CustomVerdict { tone: "mute" | "ok" | "bad"; message: string; path?: string; skills: number }

/** `profilePathHint`'s states a specimen can answer honestly, plus what the
 *  resolved folder holds. */
function customVerdict(raw: string, chosen: Provider): CustomVerdict {
  const trimmed = raw.trim();
  if (!trimmed) return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder.", skills: 0 };
  const expanded = expand(trimmed);
  if (!expanded.startsWith("/") && !/^[A-Za-z]:[\\/]/.test(expanded)) {
    return { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter.", skills: 0 };
  }
  const canned = CUSTOM[chosen.key];
  if (trimmed === canned.path) return { tone: "ok", message: `${chosen.label} profile ·`, path: expanded, skills: canned.skills };
  return { tone: "bad", message: "No folder at this path yet.", skills: 0 };
}

/** The name Add derives for a path it saves: the folder's own name, as
 *  `derived_profile_name` does. Renaming lives on the row's page. */
const folderName = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

/* ——— shared furniture ————————————————————————————————————————————— */

function attentionBadge(show: boolean) {
  if (!show) return null;
  return (
    <span className="sync-setup-state attention" aria-label="Needs review" title="Needs review">
      <Icon name="alert-triangle" size={14} />
    </span>
  );
}

function Frame({ title, closeLabel, children, footerNote, cancelLabel, actionLabel, actionIcon, ready }: {
  title: string;
  closeLabel: string;
  children: ReactNode;
  footerNote?: string | null;
  cancelLabel: string;
  actionLabel: string;
  actionIcon?: IconName;
  ready: boolean;
}) {
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page mock-page">
      <section className="profile-links-section">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label={title}>
          <header className="sync-modal-header">
            <div><h1>{title}</h1></div>
            <button type="button" className="btn btn-ghost" aria-label={closeLabel}>
              <Icon name="x" size={17} />
            </button>
          </header>
          <div className="sync-modal-body">{children}</div>
          <footer className="sync-modal-footer sync-setup-footer">
            {footerNote && <span className="sync-setup-save-state">{footerNote}</span>}
            <div>
              <button type="button" className="btn btn-ghost">{cancelLabel}</button>
              <button type="button" className="btn btn-primary" disabled={!ready}>
                {actionIcon && <Icon name={actionIcon} size={13} />}
                {actionLabel}
              </button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

type Source = "local" | "cloud";

/** New project's two doors, as Add already wears them. */
function Doors({ source, onPick, localDetail }: { source: Source; onPick: (next: Source) => void; localDetail: string }) {
  const door = (key: Source, icon: IconName, title: string, detail: string) => (
    <div className="storage-mode-option">
      <button type="button" className="storage-mode-door" role="radio" aria-checked={source === key} onClick={() => onPick(key)}>
        <span className="storage-mode-door-title"><Icon name={icon} size={17} /><strong>{title}</strong></span>
        <span className="storage-mode-door-detail">{detail}</span>
      </button>
    </div>
  );
  return (
    <div className="storage-mode-doors" role="radiogroup" aria-label="Where the resource comes from">
      {door("local", "computer", "On this computer", localDetail)}
      {door("cloud", "storage-mallard", "From Mallard Cloud", "A repository shared with you.")}
    </div>
  );
}

function CloudUnchanged() {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading"><div><strong>Storage</strong></div></div>
      <div className="sync-folder-choices-state">
        <Icon name="storage-mallard" size={14} />
        From Mallard Cloud is unchanged: the account, the repository, then the row it lands in.
      </div>
    </section>
  );
}

function Band({ title, badge, hint, children }: { title: string; badge?: ReactNode; hint?: string | null; children: ReactNode }) {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading">
        <div><strong>{title}</strong></div>
        {badge}
      </div>
      {children}
      {hint && <small className="sync-setup-hint">{hint}</small>}
    </section>
  );
}

/** The shipped option row: radio, glyph, name with a chip, a mono line, and
 *  a stamp with marks at the end. */
function OptionRow({ active, disabled, onPick, icon, title, chip, detail, meta, marks }: {
  active: boolean;
  disabled?: boolean;
  onPick: () => void;
  icon: IconName;
  title: string;
  chip?: string | null;
  detail: string;
  meta?: string | null;
  marks?: ReactNode;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      className={`sync-bundle-match-option${active ? " active" : ""}`}
      disabled={disabled}
      onClick={onPick}
    >
      <span className="sync-bundle-radio"><span /></span>
      <span className="setup-choice-mark" aria-hidden="true"><Icon name={icon} size={24} /></span>
      <span className="sync-bundle-match-copy">
        <strong>{title}{chip && <small className="sync-bundle-recommended">{chip}</small>}</strong>
        <span>{detail}</span>
      </span>
      {(meta || marks) && (
        <span className="sync-bundle-match-meta">
          {meta}
          {marks && <span className="sync-folder-choice-marks">{marks}</span>}
        </span>
      )}
    </button>
  );
}

/** New project's Storage: "None yet" leads, every storage wears its glyph. */
function StoragePills({ storages, value, onPick }: {
  storages: Storage[];
  value: string | null;
  onPick: (next: string | null) => void;
}) {
  return (
    <div className="setup-pill-row" role="radiogroup" aria-label="Storage">
      <button
        type="button"
        role="radio"
        aria-checked={value === null}
        className={`setup-pill${value === null ? " active" : ""}`}
        title="No storage linked yet — the row stays on this computer"
        onClick={() => onPick(null)}
      >
        <Icon name="computer" size={12} />
        <span className="setup-pill-label">None yet</span>
        <span className="setup-pill-check" aria-hidden="true">{value === null && <Icon name="check" size={12} />}</span>
      </button>
      {storages.map((storage) => {
        const active = value === storage.id;
        return (
          <button
            key={storage.id}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={`${storage.name} · ${storage.providerLabel}`}
            title={`${storage.name} · ${storage.providerLabel}`}
            className={`setup-pill${active ? " active" : ""}`}
            onClick={() => onPick(storage.id)}
          >
            <Icon name={storageIconName(storage.provider)} size={12} />
            <span className="setup-pill-label">{storage.name}</span>
            <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
          </button>
        );
      })}
    </div>
  );
}

function StorageBand({ scene, value, onPick }: { scene: Scenario; value: string | null; onPick: (next: string | null) => void }) {
  return (
    <Band title="Storage" hint={storageHint(scene.storages, value, scene.lastUsed)}>
      <StoragePills storages={scene.storages} value={value} onPick={onPick} />
    </Band>
  );
}

/* ——— the Agent band: one home for one kind ———————————————————————————— */

interface AgentState {
  provider: ProviderKey;
  /** A saved home, or null while a path is being typed. */
  home: Home | null;
  path: string;
}

function initialAgent(scene: Scenario, kind: Shape): AgentState {
  const chosen = defaultHome(scene, kind);
  return { provider: chosen.provider, home: chosen.home, path: "" };
}

/** What the chosen home holds of this kind, said under the row. */
function holdsLine(scene: Scenario, kind: Shape, agent: AgentState, verdict: CustomVerdict | null): string | null {
  if (agent.home) {
    const found = foundFor(scene, kind, agent.home);
    const total = foundTotal(found);
    if (total === 0) return `No ${noun(kind, 2)} here yet. Pull can bring some from a storage.`;
    const roots = found!.roots.filter((root) => (root.count ?? 0) > 0).map((root) => root.path);
    return `${total} ${noun(kind, total)} found in ${roots.join(" and ")}.`;
  }
  if (verdict?.tone === "ok") {
    const total = kind.key === "skill" ? verdict.skills : 0;
    if (total === 0) return `No ${noun(kind, 2)} here yet. Pull can bring some from a storage.`;
    return `${total} ${noun(kind, total)} found in ${CUSTOM[agent.provider].path}/skills.`;
  }
  return null;
}

/** New project's Agent band for one row: the provider pills Add already has,
 *  then the profile row — saved homes and Custom path… in one select, the
 *  inline path, Browse — with the default home chosen and a line saying what
 *  it holds. */
function AgentBand({ scene, kind, agent, onChange }: {
  scene: Scenario;
  kind: Shape;
  agent: AgentState;
  onChange: (next: AgentState) => void;
}) {
  const chosen = provider(agent.provider);
  const homes = scene.homes.filter((home) => home.provider === agent.provider);
  const pending = agent.home === null;
  const verdict = pending ? customVerdict(agent.path, chosen) : null;
  const resolved = agent.home !== null || verdict?.tone === "ok";
  const chooseProvider = (key: ProviderKey) => {
    const open = scene.homes.filter((home) => home.provider === key && !isAdded(scene, kind, home));
    const best = [...open].sort((left, right) => foundTotal(foundFor(scene, kind, right)) - foundTotal(foundFor(scene, kind, left)))[0] ?? null;
    onChange({ provider: key, home: best, path: "" });
  };
  return (
    <Band title="Agent" badge={attentionBadge(!resolved)} hint={holdsLine(scene, kind, agent, verdict)}>
      {/* Two or three registered agents is a row, not a menu — and a native
          select cannot hold the mark that tells them apart. */}
      <div className="setup-pill-row" role="radiogroup" aria-label="Provider">
        {kind.providers.map((key) => {
          const candidate = provider(key);
          const active = agent.provider === key;
          return (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={active}
              className={`setup-pill${active ? " active" : ""}`}
              onClick={() => chooseProvider(key)}
            >
              <ProviderMark provider={key} displayName={candidate.label} size={12} />
              <span className="setup-pill-label">{candidate.label}</span>
              <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
            </button>
          );
        })}
      </div>
      <div className="sync-agent-profile-list">
        <div className={`sync-agent-profile-row${pending ? " has-custom-path" : ""}`}>
          <span className="sync-select-shell">
            <select
              aria-label={`${chosen.label} profile`}
              value={pending ? "__pending" : agent.home?.id ?? ""}
              onChange={(event) => {
                const next = event.target.value;
                if (next === "__pending") onChange({ ...agent, home: null, path: "" });
                else onChange({ ...agent, home: homes.find((home) => home.id === next) ?? null, path: "" });
              }}
            >
              {homes.length === 0 && <option value="">Choose profile</option>}
              {homes.map((home) => {
                const added = isAdded(scene, kind, home);
                return (
                  <option key={home.id} value={home.id} disabled={added}>
                    {home.name}{added ? ` (already has ${kind.label})` : ""}
                  </option>
                );
              })}
              <option value="__pending">Custom path…</option>
            </select>
          </span>
          {pending && (
            <input
              value={agent.path}
              aria-label={`${chosen.label} custom profile path`}
              placeholder={`${chosen.label} profile path`}
              spellCheck={false}
              onChange={(event) => onChange({ ...agent, path: event.target.value })}
            />
          )}
          <button type="button" className="btn" onClick={() => onChange({ ...agent, home: null, path: CUSTOM[agent.provider].path })}>
            <Icon name="folder" size={13} /> Browse
          </button>
          {pending && verdict && (
            <p className={`sync-setup-path-hint ${verdict.tone}`} aria-live="polite">
              {verdict.message}
              {verdict.path && <code>{verdict.path}</code>}
            </p>
          )}
          {agent.home && (
            <p className="sync-setup-path-hint ok spans-row">
              {chosen.label} profile ·<code>{expand(agent.home.path)}</code>
            </p>
          )}
        </div>
      </div>
    </Band>
  );
}

/** "Skills · Default Codex": the row Finish makes, named before it exists. */
function rowLabel(kind: Shape, agent: AgentState): string | null {
  if (agent.home) return `${kind.label} · ${agent.home.name}`;
  const verdict = customVerdict(agent.path, provider(agent.provider));
  return verdict.tone === "ok" && verdict.path ? `${kind.label} · ${folderName(verdict.path)}` : null;
}

/* ——— Kind first ——————————————————————————————————————————————————— */

/** Step one: the kind. Found kinds lead with what they hold and where; the
 *  rest are listed under their own head, choosable for a Pull later. */
function KindStep({ scene, picked, onPick }: { scene: Scenario; picked: string | null; onPick: (key: string) => void }) {
  const [source, setSource] = useState<Source>("local");
  const kind = picked ? shape(picked) : null;
  const facts = SHAPES.map((candidate) => {
    const homes = homesFor(scene, candidate);
    const holding = homes.filter((home) => foundTotal(foundFor(scene, candidate, home)) > 0);
    const addedOn = homes.filter((home) => isAdded(scene, candidate, home));
    const total = kindTotal(scene, candidate);
    const detail = total > 0
      ? holding.map((home) => `${foundTotal(foundFor(scene, candidate, home))} in ${home.path}`).join(" · ")
      : `No ${noun(candidate, 2)} on this computer`;
    const chip = fullyAdded(scene, candidate)
      ? "Added"
      : addedOn.length > 0 ? `Added for ${addedOn.map((home) => provider(home.provider).label).join(", ")}` : null;
    const stamp = holding.map((home) => foundFor(scene, candidate, home)?.stamp ?? null).find((value) => value) ?? null;
    return { candidate, total, detail, chip, stamp, holding };
  });
  const found = facts.filter((fact) => fact.total > 0);
  const empty = facts.filter((fact) => fact.total === 0);
  const list = (rows: typeof facts, label: string) => (
    <div className="sync-bundle-match-options" role="radiogroup" aria-label={label}>
      {rows.map(({ candidate, detail, chip, stamp, holding }) => (
        <OptionRow
          key={candidate.key}
          active={picked === candidate.key}
          disabled={chip === "Added"}
          onPick={() => onPick(candidate.key)}
          icon={candidate.icon}
          title={candidate.label}
          chip={chip}
          detail={detail}
          meta={stamp}
          marks={(holding.length > 0 ? holding.map((home) => home.provider) : candidate.providers).map((key) => (
            <ProviderMark key={key} provider={key} displayName={provider(key).label} size={12} />
          ))}
        />
      ))}
    </div>
  );
  return (
    <Frame
      title="Add Catalog resource"
      closeLabel="Close; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={kind ? `Set up ${kind.label}` : "Set up"}
      ready={kind !== null}
    >
      <Doors source={source} onPick={setSource} localDetail="A kind your agent homes hold." />
      {source === "cloud" ? <CloudUnchanged /> : (
        <Band title="Kind" badge={attentionBadge(kind === null)}>
          {found.length > 0 && (
            <div className="sync-folder-choices">
              <div className="sync-folder-choices-head">Found on this computer <small>{found.length}</small></div>
              {list(found, "Kinds found on this computer")}
            </div>
          )}
          {empty.length > 0 && (
            <div className="sync-folder-choices">
              <div className="sync-folder-choices-head">Nothing here yet <small>{empty.length}</small></div>
              {list(empty, "Kinds with nothing on this computer")}
            </div>
          )}
        </Band>
      )}
    </Frame>
  );
}

/** Step two: "Set up Skills". The kind as the card you chose, then the
 *  home with its default, then the storage with its default. Finish makes
 *  the one row the footer names. */
function KindSetup({ scene, kind, onChangeKind }: { scene: Scenario; kind: Shape; onChangeKind: () => void }) {
  const [agent, setAgent] = useState(() => initialAgent(scene, kind));
  const [storage, setStorage] = useState(() => defaultStorage(scene));
  const label = rowLabel(kind, agent);
  const chosen = scene.storages.find((candidate) => candidate.id === storage) ?? null;
  return (
    <Frame
      title={`Set up ${kind.label}`}
      closeLabel="Close setup; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={chosen ? "Finish" : "Finish locally"}
      ready={label !== null}
      footerNote={label}
    >
      <Band
        title="Resource"
        badge={(
          <button type="button" className="btn btn-ghost" aria-label="Choose another kind" onClick={onChangeKind}>
            Change…
          </button>
        )}
      >
        <div className="setup-choice-grid">
          <span className="setup-choice-card active settled">
            <span className="setup-choice-card-head">
              <span className="setup-choice-mark" aria-hidden="true"><Icon name={kind.icon} size={24} /></span>
              <strong>{kind.label}</strong>
              <span className="setup-choice-card-check" aria-hidden="true"><Icon name="check" size={12} /></span>
            </span>
            <span className="setup-choice-card-foot">
              {kind.providers.map((key) => (
                <ProviderMark key={key} provider={key} displayName={provider(key).label} size={12} />
              ))}
              {kind.format} · {providerList(kind.providers.map((key) => provider(key).label))}
            </span>
          </span>
        </div>
      </Band>
      <AgentBand scene={scene} kind={kind} agent={agent} onChange={setAgent} />
      <StorageBand scene={scene} value={storage} onPick={setStorage} />
    </Frame>
  );
}

function KindFirst({ scene }: { scene: Scenario }) {
  const [picked, setPicked] = useState<string | null>(() => defaultKind(scene));
  const [revealed, setRevealed] = useState(false);
  const kind = picked ? shape(picked) : null;
  const fromDoor = scene.door !== "any" && !revealed;
  return (
    <div className="mockup-steps">
      {fromDoor ? (
        <div className="mockup-then">
          <Icon name="chevron-down" size={14} />
          Opened from the {shape(scene.door).label} door: step one is already answered, so the page opens on its setup.
        </div>
      ) : (
        <>
          <KindStep scene={scene} picked={picked} onPick={setPicked} />
          <div className="mockup-then">
            <Icon name="chevron-down" size={14} />
            {kind ? `Set up ${kind.label} opens that kind’s setup page` : "Nothing to set up until a kind is chosen"}
          </div>
        </>
      )}
      {kind && <KindSetup key={kind.key} scene={scene} kind={kind} onChangeKind={() => setRevealed(true)} />}
    </div>
  );
}

/* ——— Home first ——————————————————————————————————————————————————— */

function HomeStep({ scene, picked, onPick }: { scene: Scenario; picked: string | null; onPick: (id: string) => void }) {
  const [source, setSource] = useState<Source>("local");
  const [text, setText] = useState("");
  const home = scene.homes.find((candidate) => candidate.id === picked) ?? null;
  return (
    <Frame
      title="Add Catalog resource"
      closeLabel="Close; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={home ? `Set up ${home.name}` : "Use this home"}
      ready={home !== null}
    >
      <Doors source={source} onPick={setSource} localDetail="An agent home you have." />
      {source === "cloud" ? <CloudUnchanged /> : (
        <Band title="Agent home">
          <div className="sync-project-folder-field">
            <div className="sync-profile-select-row">
              <input
                value={text}
                placeholder={scene.homes.length > 0 ? "Path or search" : "Path, such as ~/.codex"}
                spellCheck={false}
                aria-label="Agent home"
                onChange={(event) => setText(event.target.value)}
              />
              <button type="button" className="btn"><Icon name="folder" size={13} /> Browse…</button>
            </div>
          </div>
          {scene.homes.length === 0 ? (
            <div className="sync-folder-choices-state">No Codex or Claude home found on this computer. Type a path or Browse.</div>
          ) : (
            <div className="sync-folder-choices">
              <div className="sync-folder-choices-head">Found <small>{scene.homes.length}</small></div>
              <div className="sync-bundle-match-options" role="radiogroup" aria-label="Agent homes">
                {scene.homes.map((candidate) => {
                  const kinds = SHAPES.filter((kind) => kind.providers.includes(candidate.provider));
                  const summary = kinds
                    .map((kind) => ({ kind, total: foundTotal(foundFor(scene, kind, candidate)) }))
                    .filter(({ total }) => total > 0)
                    .map(({ kind, total }) => `${total} ${noun(kind, total)}`)
                    .join(", ");
                  const inCatalog = kinds.some((kind) => isAdded(scene, kind, candidate));
                  return (
                    <OptionRow
                      key={candidate.id}
                      active={picked === candidate.id}
                      onPick={() => onPick(candidate.id)}
                      icon="folder"
                      title={candidate.name}
                      chip={inCatalog ? "In Catalog" : null}
                      detail={`${candidate.path} · ${summary || "nothing found"}`}
                      meta={candidate.lastActive}
                      marks={<ProviderMark provider={candidate.provider} displayName={provider(candidate.provider).label} size={12} />}
                    />
                  );
                })}
              </div>
            </div>
          )}
        </Band>
      )}
    </Frame>
  );
}

function HomeSetup({ scene, home }: { scene: Scenario; home: Home }) {
  const kinds = SHAPES.filter((kind) => kind.providers.includes(home.provider));
  const [name, setName] = useState(home.name);
  const [picked, setPicked] = useState<string | null>(() => (
    kinds.find((kind) => foundTotal(foundFor(scene, kind, home)) > 0 && !isAdded(scene, kind, home))?.key ?? null
  ));
  const [storage, setStorage] = useState(() => defaultStorage(scene));
  const kind = picked ? shape(picked) : null;
  const chosen = scene.storages.find((candidate) => candidate.id === storage) ?? null;
  return (
    <Frame
      title={`Set up ${name.trim() || home.name}`}
      closeLabel="Close setup; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={chosen ? "Finish" : "Finish locally"}
      ready={kind !== null}
      footerNote={kind ? `${kind.label} · ${name.trim() || home.name}` : null}
    >
      <Band title="Home">
        <div className="sync-field-grid sync-setup-project-fields">
          <label>
            <span>Name</span>
            <input value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <label>
            <span>Folder</span>
            <div className="sync-profile-select-row">
              <input value={home.path} readOnly />
              <button type="button" className="btn"><Icon name="folder" size={13} /> Browse…</button>
            </div>
          </label>
        </div>
      </Band>
      <Band title="Resource" badge={attentionBadge(kind === null)}>
        <div className="sync-bundle-match-options" role="radiogroup" aria-label="Resource kind">
          {kinds.map((candidate) => {
            const found = foundFor(scene, candidate, home);
            const total = foundTotal(found);
            const added = isAdded(scene, candidate, home);
            const roots = found?.roots.map((root) => root.path).join(", ") ?? "";
            return (
              <OptionRow
                key={candidate.key}
                active={picked === candidate.key}
                disabled={added}
                onPick={() => setPicked(candidate.key)}
                icon={candidate.icon}
                title={candidate.label}
                chip={added ? "Added" : null}
                detail={total > 0 ? `${total} ${noun(candidate, total)} · ${roots}` : `None found · ${roots}`}
                meta={found?.stamp ?? null}
              />
            );
          })}
        </div>
        {kind === null && <FieldMessage id="mock-home-first-resource">Choose one kind to continue.</FieldMessage>}
      </Band>
      <StorageBand scene={scene} value={storage} onPick={setStorage} />
    </Frame>
  );
}

function HomeFirst({ scene }: { scene: Scenario }) {
  const [picked, setPicked] = useState<string | null>(() => defaultHomeFirst(scene));
  const home = scene.homes.find((candidate) => candidate.id === picked) ?? null;
  return (
    <div className="mockup-steps">
      <HomeStep scene={scene} picked={picked} onPick={setPicked} />
      <div className="mockup-then">
        <Icon name="chevron-down" size={14} />
        {home ? `Set up ${home.name} opens that home’s setup page` : "Nothing to set up until a home is chosen"}
      </div>
      {home && <HomeSetup key={home.id} scene={scene} home={home} />}
    </div>
  );
}

/* ——— harness —————————————————————————————————————————————————————— */

function Panel({ title, note, children }: { title: string; note: ReactNode; children: ReactNode }) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function Knob<T extends string>({ label, value, options, onPick }: {
  label: string;
  value: T;
  options: { key: T; label: string }[];
  onPick: (next: T) => void;
}) {
  return (
    <div className="mockup-knob" role="radiogroup" aria-label={label}>
      <span>{label}</span>
      {options.map((option) => (
        <button
          key={option.key}
          type="button"
          role="radio"
          aria-checked={value === option.key}
          className={`btn${value === option.key ? " btn-primary" : ""}`}
          onClick={() => onPick(option.key)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

const NOTES = {
  kind: <>
    Step one asks the one question the backend cannot answer — which kind — and answers it
    anyway where it can: the kinds found on this computer lead, each with what it holds and
    where, and the first of them without its row is already chosen; the rest wait under
    “Nothing here yet”, choosable for a Pull later. The button names the step it opens. Step
    two is the project’s setup page for that kind: the card you chose, still on screen; Agent
    as today’s pills and profile row, with the home that holds the most of this kind chosen
    and a line saying what it holds; Storage as New project’s pill row with the same default
    and reason; Finish, with the row it will make named in the footer. A row already in the
    Catalog is Added in step one and greyed in the select, so the default skips to the next
    home. The sidebar’s and landing’s Kind doors are per kind, so from a door the page opens
    on step two. A second home is a second pass, and the storage you chose last is the next
    pass’s default. Recommended.
  </>,
  home: <>
    B as mocked, one row per pass: step one picks the home — the found ones with what they
    hold and the most recently active chosen, or a path — and step two sets that home up with
    one kind chosen among those it holds, the first with something in it, then Storage and
    Finish. It is New project line for line, and the right order when a home is the thing you
    think of first. But the first step asks what the backend already knew, the Kind doors have
    no step to skip to, and a kind that lives on two homes reads as two homes rather than one
    kind twice.
  </>,
};

function Harness() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    setKnobs({ ...knobs, [key]: value });
    const params = new URLSearchParams(location.search);
    params.set(key, value);
    history.replaceState(null, "", `${location.pathname}?${params.toString()}`);
  };
  const scene = scenario(knobs);
  const sceneKey = JSON.stringify(knobs);
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Add Catalog resource, one row per pass</h1>
            <p>
              New project’s two steps, with one rule: a pass sets up one Catalog row — Pets v1 on
              one home — never a batch, so every list is a radio and Finish makes exactly one row,
              as Add does today. What the page does differently is read before it asks: the kinds
              and homes this computer holds are found and pre-chosen, the home and the storage carry
              a default with its reason, and the button says what it does. The rules behind the
              defaults: pre-fill the most common value, never the first in the list (NN/g, defaults);
              a wizard’s button names the outcome and last time’s answers are next time’s defaults
              (NN/g, wizards); the common path first, the uncommon one behind Custom path…
              (progressive disclosure). Two orders below; the knobs change the machine.
            </p>
            <div className="mockup-knobs">
              <Knob label="This computer" value={knobs.stock} onPick={(next) => set("stock", next)}
                options={[{ key: "some", label: "~/.codex and ~/.claude" }, { key: "none", label: "No home" }]} />
              <Knob label="Storages" value={knobs.storages} onPick={(next) => set("storages", next)}
                options={[{ key: "none", label: "None" }, { key: "one", label: "One" }, { key: "many", label: "Three" }]} />
              <Knob label="Catalog" value={knobs.catalog} onPick={(next) => set("catalog", next)}
                options={[{ key: "empty", label: "Empty" }, { key: "some", label: "Skills · Codex added" }]} />
              <Knob label="Opened from" value={knobs.door} onPick={(next) => set("door", next)}
                options={[{ key: "any", label: "The +" }, { key: "skill", label: "Skills door" }, { key: "pet-v1", label: "Pets v1 door" }]} />
            </div>
          </div>
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              const next = !dark;
              setDark(next);
              document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
            }}
          >
            <Icon name={dark ? "sun" : "moon"} size={12} /> {dark ? "Light" : "Dark"}
          </button>
        </header>

        <Panel title="Kind first — step one picks the kind, step two sets up its home" note={NOTES.kind}>
          <KindFirst key={sceneKey} scene={scene} />
        </Panel>
        <Panel title="Home first — B as mocked, one kind per pass" note={NOTES.home}>
          <HomeFirst key={sceneKey} scene={scene} />
        </Panel>
      </div>
    </div>
  );
}

const SHELL_CSS = `
html, body { height: auto; min-height: 100%; overflow: visible; }
#root { height: auto; min-height: 100%; overflow: visible; }
.app.mockup-root { height: auto; min-height: 100vh; overflow: visible; }
.mockup-shell { width: 100%; max-width: 1180px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-knobs { display: flex; flex-wrap: wrap; gap: 8px 22px; margin-top: 14px; }
.mockup-knob { display: inline-flex; align-items: center; gap: 6px; color: var(--text-2); font-size: var(--type-xs); }
.mockup-knob > span { margin-right: 2px; font-weight: var(--weight-semibold); }
.mockup-knob .btn { min-height: 26px; padding: 0 9px; font-size: var(--type-xs); }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
.mockup-steps { display: flex; flex-direction: column; }
.mockup-then { display: flex; align-items: center; gap: 8px; padding: 10px 24px; border-top: 1px dashed var(--border); border-bottom: 1px dashed var(--border); color: var(--text-3); font-size: var(--type-xs); background: var(--bg-0); }
/* The page's own top padding stands in for the window chrome the app draws. */
.mock-page { min-height: 0; padding: 14px 0 18px; overflow: visible; }
body { background: var(--app-shell); }
`;

/* What a cutover would lift into App.css. Everything else on these pages is a
   shipped rule: the doors, the bands, the option row, the pill row, the
   profile row, the path hint, the choice card. */
const PROPOSED_CSS = `
/* The kind you chose, kept on screen in step two: the choice card, settled. */
.setup-choice-card.settled { cursor: default; }
/* A band's heading can carry a quiet verb where the badge goes. */
.sync-setup-section > .sync-card-heading > .btn-ghost { min-height: 24px; padding: 0 8px; font-size: var(--type-xs); }
`;

const START_DARK = new URLSearchParams(location.search).get("theme") === "dark";
document.documentElement.setAttribute("data-theme", START_DARK ? "dark" : "light");
const style = document.createElement("style");
style.textContent = SHELL_CSS + PROPOSED_CSS;
document.head.appendChild(style);
/* One root across hot reloads: re-running the module must reuse it, or the
   dev server's second pass warns and mounts a second tree. */
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);

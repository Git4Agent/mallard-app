import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { storageIconName, type IconName } from "./components/Icons";
import ProviderMark from "./components/sync-review/ProviderMark";
import { FieldMessage } from "./components/notice/Notice";
import "./App.css";

/**
 * Mockup: Add Catalog resource, home first, with the agent chosen the way
 * New project chooses its agents.
 *
 * The owner picked the home-first order of `catalog-add-step-mockup.html`
 * — pick the agent home, then set one kind up on it — and one thing was
 * off: that step drew the home the way New project draws a *folder* (a
 * path field, Browse, and a Found list of homes), while the project chooses
 * its *agents* on its setup page with a different part altogether: one card
 * per provider, the agent's mark and name on a toggle line, and under it
 * the profile row — saved profiles and Custom path… in one select, the
 * inline path, Browse, and a line saying what the path resolved to. That
 * band also carries the project's one discovery rule: a provider's profile
 * is preselected exactly when this machine offers one unambiguous profile
 * for it; with none, or several, it waits rather than guesses.
 *
 * Here step one is that band. One row per pass, so the toggle is a radio
 * rather than a checkbox; everything else — the select, the path, Browse,
 * the hint, the rule — is the project's, and nothing is added to it. Step
 * two is unchanged: the home, one kind, the storage, Finish.
 * The second panel is the project's own Agent band on the same data, for
 * the eye to compare.
 *
 * Shipped classes and real leaf components; static data standing in for one
 * new backend read (a count per capability root). Every panel keeps its own
 * state. Dev server only: `npm run dev`, then /catalog-add-home-mockup.html
 * (`?stock=some|none&profiles=one|two&storages=none|one|many&catalog=empty|some&door=any|skill|pet-v1&resource=cards|rows|pills&theme=dark`).
 */

/* ——— data ——————————————————————————————————————————————————————— */

type ProviderKey = "codex" | "claude";
/** `ProviderDescriptor`: the name, the profile label the card's toggle line
 *  wears, and `setup_default`. Both ship as setup defaults. */
interface Provider { key: ProviderKey; label: string; profileLabel: string }
const PROVIDERS: Provider[] = [
  { key: "codex", label: "Codex", profileLabel: ".codex profile" },
  { key: "claude", label: "Claude", profileLabel: ".claude profile" },
];
const provider = (key: ProviderKey): Provider => PROVIDERS.find((candidate) => candidate.key === key)!;

/* The registry's shapes with the backend's `format_label`, which the rows
   variant prints under the name. */
interface Shape { key: string; label: string; format: string; noun: [string, string]; icon: IconName; providers: ProviderKey[] }
const SHAPES: Shape[] = [
  { key: "skill", label: "Skills", format: "Agent Skill", noun: ["skill", "skills"], icon: "lightbulb", providers: ["codex", "claude"] },
  { key: "plugin", label: "Plugins", format: "Plugin install intent", noun: ["plugin", "plugins"], icon: "catalog-plugin", providers: ["codex", "claude"] },
  { key: "pet-v1", label: "Pets v1", format: "Codex pet v1", noun: ["pet", "pets"], icon: "catalog-pet-v1", providers: ["codex"] },
  { key: "pet-v2", label: "Pets v2", format: "Codex pet v2", noun: ["pet", "pets"], icon: "catalog-pet-v2", providers: ["codex"] },
];
const shape = (key: string): Shape => SHAPES.find((candidate) => candidate.key === key)!;
const noun = (kind: Shape, count: number) => kind.noun[count === 1 ? 0 : 1];

/* Saved profiles, as both `list_provider_profiles` and
   `list_catalog_resource_profiles` list them: the defaults are registered by
   the backend when ~/.codex and ~/.claude exist, and named "Default Codex"
   and "Default Claude" by both. A second Codex profile is the knob's. */
interface Profile { id: string; provider: ProviderKey; name: string; path: string; lastActive: string }
const DEFAULTS: Profile[] = [
  { id: "codex-default", provider: "codex", name: "Default Codex", path: "~/.codex", lastActive: "2 hours ago" },
  { id: "claude-default", provider: "claude", name: "Default Claude", path: "~/.claude", lastActive: "yesterday" },
];
const SECOND_CODEX: Profile = { id: "codex-work", provider: "codex", name: "Work Codex", path: "~/work/.codex", lastActive: "last week" };
const HOME_DIR = "/Users/you";
const expand = (path: string) => (path === "~" ? HOME_DIR : path.startsWith("~/") ? `${HOME_DIR}/${path.slice(2)}` : path);

/* What the one new read answers, per shape per profile: the roots the
   capability would scan under that home and how many entries each holds
   (null: the root does not exist), plus the newest entry's stamp. A readdir
   per root, bounded and no-follow — never an inventory, never a hash. */
interface Found { shape: string; profile: string; roots: { path: string; count: number | null }[]; stamp: string | null }
const FOUND: Found[] = [
  { shape: "skill", profile: "codex-default", roots: [{ path: "~/.codex/skills", count: 9 }, { path: "~/.agents/skills", count: 3 }], stamp: "2 hours ago" },
  { shape: "plugin", profile: "codex-default", roots: [{ path: "~/.codex/plugins", count: 3 }], stamp: "3 days ago" },
  { shape: "pet-v1", profile: "codex-default", roots: [{ path: "~/.codex/pets", count: 0 }], stamp: null },
  { shape: "pet-v2", profile: "codex-default", roots: [{ path: "~/.codex/pets", count: 2 }], stamp: "last week" },
  { shape: "skill", profile: "claude-default", roots: [{ path: "~/.claude/skills", count: 4 }], stamp: "yesterday" },
  { shape: "plugin", profile: "claude-default", roots: [{ path: "~/.claude/plugins", count: null }], stamp: null },
  { shape: "skill", profile: "codex-work", roots: [{ path: "~/work/.codex/skills", count: 1 }], stamp: "last week" },
  { shape: "plugin", profile: "codex-work", roots: [{ path: "~/work/.codex/plugins", count: null }], stamp: null },
  { shape: "pet-v1", profile: "codex-work", roots: [{ path: "~/work/.codex/pets", count: null }], stamp: null },
  { shape: "pet-v2", profile: "codex-work", roots: [{ path: "~/work/.codex/pets", count: null }], stamp: null },
];
const foundTotal = (found: Found | undefined) => found?.roots.reduce((sum, root) => sum + (root.count ?? 0), 0) ?? 0;

/* Storages in the order both setup pages list them: Mallard Cloud, then the
   other clouds, then local folders — the owner's preference, which is also
   the default's order when nothing was used before. */
interface Storage { id: string; name: string; provider: string; providerLabel: string }
const STORAGES: Storage[] = [
  { id: "cloud", name: "Mallard Cloud", provider: "managed_cloud", providerLabel: "Mallard Cloud · kai" },
  { id: "vault", name: "Vault", provider: "r2", providerLabel: "Cloudflare R2" },
  { id: "backup", name: "Backup drive", provider: "local", providerLabel: "Local folder" },
];

/* The canned machine behind Custom path…: one folder per provider answers,
   holding a couple of skills and nothing else. */
const CUSTOM: Record<ProviderKey, { path: string; skills: number }> = {
  codex: { path: "~/agents/.codex", skills: 1 },
  claude: { path: "~/work/.claude", skills: 2 },
};

/* ——— knobs ———————————————————————————————————————————————————————— */

type Stock = "some" | "none";
type ProfileStock = "one" | "two";
type StorageStock = "none" | "one" | "many";
type CatalogStock = "empty" | "some";
type Door = "any" | "skill" | "pet-v1";
/** How step two draws the kind: the shipped Kind cards, tidied rows, or pills. */
type ResourceStyle = "cards" | "rows" | "pills";
interface Knobs { stock: Stock; profiles: ProfileStock; storages: StorageStock; catalog: CatalogStock; door: Door; resource: ResourceStyle }

function readKnobs(): Knobs {
  const params = new URLSearchParams(location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = params.get(key);
    return allowed.includes(value as T) ? (value as T) : fallback;
  };
  return {
    stock: pick("stock", ["some", "none"], "some"),
    profiles: pick("profiles", ["one", "two"], "one"),
    storages: pick("storages", ["none", "one", "many"], "one"),
    catalog: pick("catalog", ["empty", "some"], "empty"),
    door: pick("door", ["any", "skill", "pet-v1"], "any"),
    resource: pick("resource", ["cards", "rows", "pills"], "cards"),
  };
}

/** One machine, as the knobs describe it. */
interface Scenario {
  profiles: Profile[];
  found: Found[];
  storages: Storage[];
  /** Rows already in the Catalog, as "shape:profile". */
  added: Set<string>;
  /** The storage a Catalog row used last, when one did. */
  lastUsed: string | null;
  /** The Kind door the page was opened from, when one was. */
  door: Door;
}

function scenario(knobs: Knobs): Scenario {
  const stocked = knobs.stock === "some";
  const profiles = stocked ? (knobs.profiles === "two" ? [DEFAULTS[0]!, SECOND_CODEX, DEFAULTS[1]!] : DEFAULTS) : [];
  return {
    profiles,
    found: FOUND.filter((found) => profiles.some((candidate) => candidate.id === found.profile)),
    storages: knobs.storages === "none" ? [] : knobs.storages === "one" ? STORAGES.slice(1, 2) : STORAGES,
    added: new Set(knobs.catalog === "some" ? ["skill:codex-default"] : []),
    lastUsed: knobs.catalog === "some" && knobs.storages !== "none" ? "vault" : null,
    door: knobs.door,
  };
}

const profilesOf = (scene: Scenario, key: ProviderKey) => scene.profiles.filter((candidate) => candidate.provider === key);
const foundFor = (scene: Scenario, kind: Shape, profileId: string) =>
  scene.found.find((found) => found.shape === kind.key && found.profile === profileId);

/* ——— the defaults ————————————————————————————————————————————————— */

/** New project's rule, verbatim: a setup-default provider's profile is
 *  preselected exactly when this machine offers one unambiguous profile for
 *  it. None, or several: nothing is guessed. */
function unambiguousProfile(scene: Scenario, key: ProviderKey): Profile | null {
  const candidates = profilesOf(scene, key);
  return candidates.length === 1 ? candidates[0]! : null;
}

/** Which card starts chosen: the provider whose profile was active most
 *  recently; the first provider when this machine has none. */
function defaultProvider(scene: Scenario): ProviderKey {
  return scene.profiles[0]?.provider ?? PROVIDERS[0]!.key;
}

/** The storage the row starts on: the one a Catalog row used last, else the
 *  first by preference — Mallard Cloud, other clouds, local — else none. */
function defaultStorage(scene: Scenario): string | null {
  if (scene.lastUsed && scene.storages.some((storage) => storage.id === scene.lastUsed)) return scene.lastUsed;
  return scene.storages[0]?.id ?? null;
}

/** Why the default is the default, in a few words; nothing otherwise. */
function storageHint(storages: Storage[], value: string | null, lastUsed: string | null): string | null {
  const chosen = storages.find((storage) => storage.id === value);
  if (!chosen) return storages.length === 0 ? "No storage yet." : null;
  if (chosen.id === lastUsed) return "Used last by the Catalog.";
  if (storages.length === 1) return "Your only storage.";
  if (chosen.id !== storages[0]?.id) return null;
  return chosen.provider === "managed_cloud" ? "Mallard Cloud first." : chosen.provider === "local" ? null : "Cloud before local.";
}

/* ——— the custom path ——————————————————————————————————————————————— */

interface PathHint { tone: "mute" | "ok" | "bad"; message: string; path?: string }

/** `profilePathHint`'s states a specimen can answer honestly. */
function pathHint(raw: string, chosen: Provider): PathHint {
  const trimmed = raw.trim();
  if (!trimmed) return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder." };
  const expanded = expand(trimmed);
  if (!expanded.startsWith("/") && !/^[A-Za-z]:[\\/]/.test(expanded)) {
    return { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter." };
  }
  if (trimmed === CUSTOM[chosen.key].path) return { tone: "ok", message: `${chosen.label} profile ·`, path: expanded };
  return { tone: "bad", message: "No folder at this path yet." };
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

function Frame({ title, closeLabel, children, footerNote, cancelLabel, actionLabel, ready }: {
  title: string;
  closeLabel: string;
  children: ReactNode;
  footerNote?: string | null;
  cancelLabel: string;
  actionLabel: string;
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
              <button type="button" className="btn btn-primary" disabled={!ready}>{actionLabel}</button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

type Source = "local" | "cloud";

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

/* ——— the kind, three ways ———————————————————————————————————————— */

interface KindRow { kind: Shape; total: number; roots: string; stamp: string | null; added: boolean }

/** What the chosen home holds of each kind it can hold: the count, the
 *  roots the count came from, the newest stamp, and whether the row exists. */
function kindRows(scene: Scenario, home: ChosenHome): KindRow[] {
  return SHAPES.filter((kind) => kind.providers.includes(home.provider)).map((kind) => {
    const found = home.profile ? foundFor(scene, kind, home.profile.id) : undefined;
    const customTotal = !home.profile && kind.key === "skill" ? CUSTOM[home.provider].skills : 0;
    const total = home.profile ? foundTotal(found) : customTotal;
    const roots = home.profile
      ? found?.roots.filter((root) => root.count !== null).map((root) => root.path).join(", ") ?? ""
      : `${CUSTOM[home.provider].path}/${kind.key === "skill" ? "skills" : kind.key === "plugin" ? "plugins" : "pets"}`;
    const added = home.profile ? scene.added.has(`${kind.key}:${home.profile.id}`) : false;
    return { kind, total, roots, stamp: found?.stamp ?? null, added };
  });
}

const countOf = (row: KindRow) => `${row.total} ${noun(row.kind, row.total)}`;

/** One kind, chosen one of three ways. Cards are the shipped Kind picker —
 *  the landing door, filled — with the count where the door wears its
 *  marks. Rows are the option list with the paths gone: the format under
 *  the name, the count and the stamp at the end. Pills are the Storage
 *  row's grammar. The paths live in the tooltip in every variant. */
function ResourceChoice({ style, rows, picked, onPick }: {
  style: ResourceStyle;
  rows: KindRow[];
  picked: string | null;
  onPick: (key: string) => void;
}) {
  if (style === "cards") {
    return (
      <div className="setup-choice-grid" role="radiogroup" aria-label="Resource kind">
        {rows.map((row) => {
          const active = picked === row.kind.key;
          return (
            <button
              key={row.kind.key}
              type="button"
              role="radio"
              aria-checked={active}
              className={`setup-choice-card${active ? " active" : ""}`}
              disabled={row.added}
              title={row.roots || undefined}
              onClick={() => onPick(row.kind.key)}
            >
              <span className="setup-choice-card-head">
                <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.kind.icon} size={24} /></span>
                <strong>{row.kind.label}{row.added && <small className="sync-bundle-recommended">Added</small>}</strong>
                <span className="setup-choice-card-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
              </span>
              <span className="setup-choice-card-foot">
                {row.total > 0 ? `${countOf(row)}${row.stamp ? ` · ${row.stamp}` : ""}` : "Nothing here yet"}
              </span>
            </button>
          );
        })}
      </div>
    );
  }
  if (style === "rows") {
    return (
      <div className="sync-bundle-match-options setup-kind-rows" role="radiogroup" aria-label="Resource kind">
        {rows.map((row) => {
          const active = picked === row.kind.key;
          return (
            <button
              key={row.kind.key}
              type="button"
              role="radio"
              aria-checked={active}
              className={`sync-bundle-match-option${active ? " active" : ""}`}
              disabled={row.added}
              title={row.roots || undefined}
              onClick={() => onPick(row.kind.key)}
            >
              <span className="sync-bundle-radio"><span /></span>
              <span className="setup-choice-mark" aria-hidden="true"><Icon name={row.kind.icon} size={24} /></span>
              <span className="sync-bundle-match-copy">
                <strong>{row.kind.label}{row.added && <small className="sync-bundle-recommended">Added</small>}</strong>
                <span>{row.kind.format}</span>
              </span>
              <span className="sync-bundle-match-meta">
                {row.total > 0 ? countOf(row) : "None yet"}
                {row.stamp && <small>{row.stamp}</small>}
              </span>
            </button>
          );
        })}
      </div>
    );
  }
  return (
    <div className="setup-pill-row" role="radiogroup" aria-label="Resource kind">
      {rows.map((row) => {
        const active = picked === row.kind.key;
        return (
          <button
            key={row.kind.key}
            type="button"
            role="radio"
            aria-checked={active}
            className={`setup-pill${active ? " active" : ""}`}
            disabled={row.added}
            title={row.added ? "Added" : row.roots || undefined}
            onClick={() => onPick(row.kind.key)}
          >
            <Icon name={row.kind.icon} size={12} />
            <span className="setup-pill-label">{row.kind.label}{row.total > 0 ? ` · ${row.total}` : ""}</span>
            <span className="setup-pill-check" aria-hidden="true">{active && <Icon name="check" size={12} />}</span>
          </button>
        );
      })}
    </div>
  );
}

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

/* ——— the profile row: New project's, verbatim ———————————————————————— */

/** One provider's selection: a saved profile, or a path being typed. */
type Selection =
  | { kind: "none" }
  | { kind: "existing"; profile: Profile }
  | { kind: "pending"; path: string };

/** `profileRow` from ProjectSetupWorkspace: the select that holds saved
 *  profiles and Custom path…, the inline path when one is being typed,
 *  Browse, and the hint line under them. `claimed` greys a profile the way
 *  "(used by …)" does on the project page. */
function ProfileRow({ descriptor, profiles, selection, claimed, incomplete, errorId, onChange }: {
  descriptor: Provider;
  profiles: Profile[];
  selection: Selection;
  /** Why a saved profile cannot be chosen here, by profile id. */
  claimed?: Map<string, string>;
  incomplete: boolean;
  errorId?: string;
  onChange: (next: Selection) => void;
}) {
  const label = descriptor.label;
  const pending = selection.kind === "pending";
  const hint = pending ? pathHint(selection.path, descriptor) : null;
  const hintId = `sync-setup-path-hint-${descriptor.key}`;
  const value = selection.kind === "existing" ? selection.profile.id : pending ? "__pending" : "";
  return (
    <div className={`sync-agent-profile-row${pending ? " has-custom-path" : ""}${incomplete ? " has-error" : ""}`}>
      <span className="sync-select-shell">
        <select
          value={value}
          aria-label={`${label} profile`}
          aria-invalid={incomplete || undefined}
          aria-describedby={incomplete ? errorId : undefined}
          onChange={(event) => {
            const next = event.target.value;
            if (next === "__pending") onChange({ kind: "pending", path: "" });
            else if (!next) onChange({ kind: "none" });
            else onChange({ kind: "existing", profile: profiles.find((profile) => profile.id === next)! });
          }}
        >
          <option value="">Choose profile</option>
          {profiles.map((profile) => {
            const reason = claimed?.get(profile.id);
            return (
              <option key={profile.id} value={profile.id} disabled={Boolean(reason)}>
                {profile.name}{reason ? ` (${reason})` : ""}
              </option>
            );
          })}
          <option value="__pending">Custom path…</option>
        </select>
      </span>
      {pending && (
        <input
          value={selection.path}
          aria-label={`${label} custom profile path`}
          aria-invalid={hint?.tone === "bad" || undefined}
          aria-describedby={[incomplete ? errorId : null, hintId].filter(Boolean).join(" ") || undefined}
          placeholder={`${label} profile path`}
          spellCheck={false}
          onChange={(event) => onChange({ kind: "pending", path: event.target.value })}
        />
      )}
      <button type="button" className="btn" onClick={() => onChange({ kind: "pending", path: CUSTOM[descriptor.key].path })}>
        <Icon name="folder" size={13} /> Browse
      </button>
      {hint && (
        <p id={hintId} className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">
          {hint.message}
          {hint.path && <code>{hint.path}</code>}
        </p>
      )}
      {selection.kind === "existing" && (
        <p className="sync-setup-path-hint ok spans-row">
          {label} profile ·<code>{expand(selection.profile.path)}</code>
        </p>
      )}
    </div>
  );
}

/* ——— step one: the home, as the project's Agent band ———————————————— */

/** What step one resolved to: a saved profile, or a path Add will save. */
interface ChosenHome { provider: ProviderKey; profile: Profile | null; name: string; path: string }

function resolveHome(selection: Selection, key: ProviderKey): ChosenHome | null {
  if (selection.kind === "existing") {
    return { provider: key, profile: selection.profile, name: selection.profile.name, path: expand(selection.profile.path) };
  }
  if (selection.kind === "pending") {
    const hint = pathHint(selection.path, provider(key));
    if (hint.tone === "ok" && hint.path) return { provider: key, profile: null, name: folderName(hint.path), path: hint.path };
  }
  return null;
}

function HomeStep({ scene, chosen, onChoose }: {
  scene: Scenario;
  chosen: { provider: ProviderKey; selection: Selection };
  onChoose: (next: { provider: ProviderKey; selection: Selection }) => void;
}) {
  const [source, setSource] = useState<Source>("local");
  const home = resolveHome(chosen.selection, chosen.provider);
  const descriptor = provider(chosen.provider);
  const incomplete = home === null
    && (chosen.selection.kind === "none" || (chosen.selection.kind === "pending" && !chosen.selection.path.trim()));
  const errorMessage = incomplete ? `Choose a profile for ${descriptor.label}.` : null;
  return (
    <Frame
      title="Add Catalog resource"
      closeLabel="Close; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={home ? `Set up ${home.name}` : "Set up"}
      ready={home !== null}
    >
      <Doors source={source} onPick={setSource} localDetail="An agent home you have." />
      {source === "cloud" ? <CloudUnchanged /> : (
        <Band title="Agent" badge={attentionBadge(home === null)}>
          {/* The project's provider cards. One row per pass, so the toggle
              is a radio: the card you choose opens its profile row; the
              other is just its name. */}
          <div className="sync-provider-cards" role="radiogroup" aria-label="Agent home for this resource">
            {PROVIDERS.map((candidate) => {
              const active = chosen.provider === candidate.key;
              const profiles = profilesOf(scene, candidate.key);
              return (
                <div key={candidate.key} className={`sync-provider-card${active ? " enabled" : ""}`}>
                  <label className="sync-provider-toggle setup-marked-toggle">
                    <input
                      type="radio"
                      name="catalog-home-provider"
                      checked={active}
                      aria-label={`Set up a ${candidate.label} home`}
                      onChange={() => {
                        const only = unambiguousProfile(scene, candidate.key);
                        onChoose({ provider: candidate.key, selection: only ? { kind: "existing", profile: only } : { kind: "none" } });
                      }}
                    />
                    <ProviderMark provider={candidate.key} displayName={candidate.label} size={12} />
                    <strong>{candidate.label}</strong>
                    <span>{candidate.profileLabel}</span>
                  </label>
                  {active && (
                    <div className="sync-agent-profile-list">
                      <ProfileRow
                        descriptor={candidate}
                        profiles={profiles}
                        selection={chosen.selection}
                        incomplete={incomplete}
                        errorId="sync-setup-error-agent"
                        onChange={(selection) => onChoose({ provider: candidate.key, selection })}
                      />
                    </div>
                  )}
                </div>
              );
            })}
          </div>
          {errorMessage && <FieldMessage id="sync-setup-error-agent">{errorMessage}</FieldMessage>}
        </Band>
      )}
    </Frame>
  );
}

/* ——— step two: one kind on that home ———————————————————————————————— */

function HomeSetup({ scene, home, style }: { scene: Scenario; home: ChosenHome; style: ResourceStyle }) {
  const rows = kindRows(scene, home);
  const [name, setName] = useState(home.name);
  const [picked, setPicked] = useState<string | null>(() => {
    if (scene.door !== "any" && rows.some((row) => row.kind.key === scene.door && !row.added)) return scene.door;
    return rows.find((row) => row.total > 0 && !row.added)?.kind.key ?? null;
  });
  const [storage, setStorage] = useState(() => defaultStorage(scene));
  const kind = picked ? shape(picked) : null;
  const chosen = scene.storages.find((candidate) => candidate.id === storage) ?? null;
  const shownName = name.trim() || home.name;
  return (
    <Frame
      title={`Set up ${shownName}`}
      closeLabel="Close setup; nothing has been saved"
      cancelLabel="Cancel"
      actionLabel={chosen ? "Finish" : "Finish locally"}
      ready={kind !== null}
      footerNote={kind ? `${kind.label} · ${shownName}` : null}
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
        <ResourceChoice style={style} rows={rows} picked={picked} onPick={setPicked} />
        {kind === null && <FieldMessage id="mock-home-resource">Choose a kind.</FieldMessage>}
      </Band>
      <Band title="Storage" hint={storageHint(scene.storages, storage, scene.lastUsed)}>
        <StoragePills storages={scene.storages} value={storage} onPick={setStorage} />
      </Band>
    </Frame>
  );
}

function HomeFirst({ scene, style }: { scene: Scenario; style: ResourceStyle }) {
  const [chosen, setChosen] = useState<{ provider: ProviderKey; selection: Selection }>(() => {
    const key = defaultProvider(scene);
    const only = unambiguousProfile(scene, key);
    return { provider: key, selection: only ? { kind: "existing", profile: only } : { kind: "none" } };
  });
  const home = resolveHome(chosen.selection, chosen.provider);
  const homeKey = home ? `${home.provider}:${home.profile?.id ?? home.path}` : "none";
  return (
    <div className="mockup-steps">
      <HomeStep scene={scene} chosen={chosen} onChoose={setChosen} />
      <div className="mockup-then">
        <Icon name="chevron-down" size={14} />
        {home
          ? `Then: Set up ${home.name}${scene.door !== "any" ? ` · ${shape(scene.door).label} chosen from the door` : ""}`
          : "Then: nothing until a profile is chosen"}
      </div>
      {home && <HomeSetup key={homeKey} scene={scene} home={home} style={style} />}
    </div>
  );
}

/* ——— reference: the project's Agent band, as it ships ———————————————— */

function ProjectReference({ scene }: { scene: Scenario }) {
  /* New project's rule applied to the same machine: every setup-default
     provider on, each with its one unambiguous profile — or none chosen. */
  const [selections, setSelections] = useState<Record<ProviderKey, Selection>>(() => ({
    codex: unambiguousProfile(scene, "codex") ? { kind: "existing", profile: unambiguousProfile(scene, "codex")! } : { kind: "none" },
    claude: unambiguousProfile(scene, "claude") ? { kind: "existing", profile: unambiguousProfile(scene, "claude")! } : { kind: "none" },
  }));
  const [enabled, setEnabled] = useState<Set<ProviderKey>>(() => new Set<ProviderKey>(["codex", "claude"]));
  const incomplete = PROVIDERS.filter((candidate) => enabled.has(candidate.key) && selections[candidate.key].kind === "none");
  const message = enabled.size === 0
    ? "Turn on at least one agent to continue."
    : incomplete.length > 0 ? `Choose a profile for ${incomplete.map((candidate) => candidate.label).join(", ")}.` : null;
  return (
    <Frame
      title="Set up HorrorGame"
      closeLabel="Close setup; the draft is kept"
      cancelLabel="Discard"
      actionLabel="Finish locally"
      ready={message === null}
    >
      <Band title="Project">
        <div className="sync-field-grid sync-setup-project-fields">
          <label><span>Name</span><input value="HorrorGame" readOnly /></label>
          <label>
            <span>Folder</span>
            <div className="sync-profile-select-row">
              <input value="~/Projects/HorrorGame" readOnly />
              <button type="button" className="btn"><Icon name="folder" size={13} /> Browse…</button>
            </div>
          </label>
        </div>
      </Band>
      <Band title="Agent" badge={attentionBadge(message !== null)}>
        <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
          {PROVIDERS.map((candidate) => {
            const on = enabled.has(candidate.key);
            return (
              <div key={candidate.key} className={`sync-provider-card${on ? " enabled" : ""}`}>
                <label className="sync-provider-toggle setup-marked-toggle">
                  <input
                    type="checkbox"
                    checked={on}
                    disabled={on && enabled.size === 1}
                    aria-label={`Use ${candidate.label} in this project`}
                    onChange={() => setEnabled((current) => {
                      const next = new Set(current);
                      if (next.has(candidate.key)) next.delete(candidate.key); else next.add(candidate.key);
                      return next;
                    })}
                  />
                  <ProviderMark provider={candidate.key} displayName={candidate.label} size={12} />
                  <strong>{candidate.label}</strong>
                  <span>{candidate.profileLabel}</span>
                </label>
                {on ? (
                  <div className="sync-agent-profile-list">
                    <ProfileRow
                      descriptor={candidate}
                      profiles={profilesOf(scene, candidate.key)}
                      selection={selections[candidate.key]}
                      incomplete={selections[candidate.key].kind === "none"}
                      errorId="sync-setup-error-agent-reference"
                      onChange={(selection) => setSelections((current) => ({ ...current, [candidate.key]: selection }))}
                    />
                  </div>
                ) : (
                  <small>Not used by this project on this machine.</small>
                )}
              </div>
            );
          })}
        </div>
        {message && <FieldMessage id="sync-setup-error-agent-reference">{message}</FieldMessage>}
      </Band>
      <section className="sync-setup-section">
        <div className="sync-folder-choices-state">Storage and Repository follow, as the project page draws them.</div>
      </section>
    </Frame>
  );
}

/* ——— the band alone, three ways ———————————————————————————————————— */

const WAYS: { style: ResourceStyle; title: string; note: string }[] = [
  { style: "cards", title: "Cards", note: "The shipped Kind picker — the landing door, filled — with the count where the door wears its marks. Recommended: the kind is chosen the same way on every page." },
  { style: "rows", title: "Rows", note: "Today’s rows with the paths gone: the format under the name, the count and the stamp at the end." },
  { style: "pills", title: "Pills", note: "The Storage row’s grammar: one line, glyph, name, count. Smallest — and the kind’s picture is smallest too." },
];

function ResourceWays({ scene }: { scene: Scenario }) {
  const profile = scene.profiles[0] ?? null;
  const home: ChosenHome = profile
    ? { provider: profile.provider, profile, name: profile.name, path: expand(profile.path) }
    : { provider: "codex", profile: null, name: ".codex", path: expand(CUSTOM.codex.path) };
  const rows = kindRows(scene, home);
  const [picked, setPicked] = useState<string | null>(rows.find((row) => row.total > 0 && !row.added)?.kind.key ?? null);
  return (
    <div className="mockup-ways">
      {WAYS.map((way) => (
        <div key={way.style} className="mockup-way">
          <h4>{way.title}</h4>
          <p>{way.note}</p>
          <div className="mockup-stage">
            <main className="sync-main sync-project-links-page sync-project-setup-page mock-page">
              <section className="profile-links-section">
                <section className="sync-inline-new-project sync-setup-workspace" aria-label={`Resource band, ${way.title.toLowerCase()}`}>
                  <div className="sync-modal-body">
                    <Band title="Resource">
                      <ResourceChoice style={way.style} rows={rows} picked={picked} onPick={setPicked} />
                    </Band>
                  </div>
                </section>
              </section>
            </main>
          </div>
        </div>
      ))}
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
  home: <>
    Step one is the project’s Agent band, lifted whole: one card per provider with its mark,
    name and profile label on the toggle line; the chosen card opens the project’s profile row
    — saved profiles and Custom path… in one select, the inline path, Browse, and the hint
    naming what the path resolved to. Two things differ, both because a pass makes one row:
    the toggle is a radio rather than a checkbox, and the card not in use is only its name. The
    card that starts chosen is the provider whose home was active most
    recently, and its profile is preselected by the project’s own rule — exactly one profile
    on this computer, else “Choose profile” and the same “Choose a profile for Codex.” line
    the project shows. Nothing is added to the band; what a home holds is step two’s list. The
    button names the step it opens. Step two is as before: the home, one kind chosen among
    those it holds, the storage, Finish.
  </>,
  ways: <>
    The same band on Default Codex, drawn three ways; the knob above puts any of them in the
    flow. The paths the rows used to print repeat the home chosen a step earlier, so every
    variant moves them to the tooltip and keeps the count, which is the fact that decides.
  </>,
  reference: <>
    New project’s setup page on the same machine — the Project band and the Agent band as
    they ship, with the same two profiles, the same rule and the same words — so the two
    panels can be read against each other. The project switches every default agent on and
    keeps the checkbox; the Catalog chooses one and keeps the rest of the part.
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
            <h1>Add Catalog resource, home first — the agent as the project’s</h1>
            <p>
              Home first, one row per pass, as picked. The earlier specimen drew the home the way
              New project draws a folder: a path field, Browse and a Found list. The project
              chooses its agents with a different part — provider cards over the profile row — and
              with one rule: preselect a provider’s profile only when this computer offers exactly
              one, never guess among several. Step one now is that part and that rule. The knobs
              change the machine; “two” gives Codex a second profile, which is the case the rule
              exists for.
            </p>
            <div className="mockup-knobs">
              <Knob label="This computer" value={knobs.stock} onPick={(next) => set("stock", next)}
                options={[{ key: "some", label: "~/.codex and ~/.claude" }, { key: "none", label: "No home" }]} />
              <Knob label="Codex profiles" value={knobs.profiles} onPick={(next) => set("profiles", next)}
                options={[{ key: "one", label: "One" }, { key: "two", label: "Two" }]} />
              <Knob label="Storages" value={knobs.storages} onPick={(next) => set("storages", next)}
                options={[{ key: "none", label: "None" }, { key: "one", label: "One" }, { key: "many", label: "Three" }]} />
              <Knob label="Catalog" value={knobs.catalog} onPick={(next) => set("catalog", next)}
                options={[{ key: "empty", label: "Empty" }, { key: "some", label: "Skills · Codex added" }]} />
              <Knob label="Opened from" value={knobs.door} onPick={(next) => set("door", next)}
                options={[{ key: "any", label: "The +" }, { key: "skill", label: "Skills door" }, { key: "pet-v1", label: "Pets v1 door" }]} />
              <Knob label="Resource band" value={knobs.resource} onPick={(next) => set("resource", next)}
                options={[{ key: "cards", label: "Cards" }, { key: "rows", label: "Rows" }, { key: "pills", label: "Pills" }]} />
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

        <Panel title="Add Catalog resource — home first, agent chosen as the project chooses its agents" note={NOTES.home}>
          <HomeFirst key={sceneKey} scene={scene} style={knobs.resource} />
        </Panel>
        <Panel title="Step two’s Resource band, three ways" note={NOTES.ways}>
          <ResourceWays key={sceneKey} scene={scene} />
        </Panel>
        <Panel title="Reference — New project’s setup page, as it ships" note={NOTES.reference}>
          <ProjectReference key={sceneKey} scene={scene} />
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
.mockup-ways { display: grid; gap: 18px; }
.mockup-way > h4 { margin: 0 0 3px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-md); font-weight: 400; }
.mockup-way > p { max-width: 88ch; margin: 0 0 10px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.5; }
body { background: var(--app-shell); }
`;

/* What a cutover would lift into App.css: the rows variant prints a format
   label, not a path, under the name, so it takes the text face. */
const PROPOSED_CSS = `
.setup-kind-rows .sync-bundle-match-copy span { font-family: var(--font); }
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

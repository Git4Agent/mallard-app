import { StrictMode, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon, { type IconName } from "../src/components/Icons";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import "../src/App.css";

/**
 * Mockup: the two setup pages — "Add Catalog resource" and "Set up <project>"
 * — with icons, designed together.
 *
 * Both pages ask the reader to pick things the rest of the app already draws.
 * The Catalog landing draws each Kind as a door wearing its glyph (lightbulb
 * for Skills, puzzle for Plugins, paw and cat for the two pet formats); the
 * Storages landing draws each storage with its own hint glyph; every sync row
 * wears a ProviderMark for Codex or Claude. The setup pages are where all of
 * that stops: a bullet radio, a checkbox, and two native <select>s, which
 * cannot hold a mark at all. The picture you clicked disappears exactly when
 * you are asked to confirm it.
 *
 * Three directions, applied to BOTH pages so they stay one system. Each is
 * ordered by how much work the icons do:
 *
 *   A · Marked rows      identity only. Same bands, same controls; every
 *                        choice wears its glyph or mark, and the <select>s
 *                        that hold a short list become marked pills.
 *   B · Cards            the icons become the picker. The short list that
 *                        deserves a picture — Catalog Kind, project Storage —
 *                        becomes a card grid in the landing's own anatomy.
 *   C · Steps and result identity plus state plus outcome. Numbered steps
 *                        whose markers close as they are answered, and a
 *                        result card naming exactly what the button will make.
 *
 * The two pages share every part: one Frame, one choice row, one card, one
 * pill row, one step marker, one result card, and one profile row. Whatever
 * is picked lands on both pages at once.
 *
 * The profile row is the project setup page's own, everywhere, custom path
 * and all: the reader may not have a saved home yet, and a pasted path or an
 * unmounted volume has no way through a folder picker.
 *
 * Static data, shipped classes where they exist, real leaf components. Every
 * panel keeps its own state, so each can be driven independently.
 * Dev server only: `npm run dev`, then /catalog-setup-mockup.html.
 */

/* ——— data ————————————————————————————————————————————————————————— */

const PROVIDER_LABEL: Record<string, string> = { codex: "Codex", claude: "Claude" };
const ALL_PROVIDERS = ["codex", "claude"];

interface Kind {
  key: string;
  label: string;
  icon: IconName;
  providers: string[];
}

/* The registry this build would register: the four shapes the Catalog ships
   glyphs for, in the backend's order. */
const KINDS: Kind[] = [
  { key: "skill/dir@1", label: "Skills", icon: "lightbulb", providers: ["codex", "claude"] },
  { key: "plugin/dir@1", label: "Plugins", icon: "catalog-plugin", providers: ["claude"] },
  { key: "pet/json@1", label: "Pets", icon: "catalog-pet-v1", providers: ["codex", "claude"] },
  { key: "pet/json@2", label: "Pets v2", icon: "catalog-pet-v2", providers: ["codex"] },
];

const PROFILES = [
  { id: "codex-home", provider: "codex", name: "Codex home", path: "~/.codex" },
  { id: "claude-home", provider: "claude", name: "Claude home", path: "~/.claude" },
  { id: "claude-work", provider: "claude", name: "Claude (work)", path: "~/work/.claude" },
];

/* `ProviderDescriptor.default_profile_directory`: what the backend's probe
   looks for one level down when you hand it a parent folder. */
const DEFAULT_DIR: Record<string, string> = { codex: ".codex", claude: ".claude" };

/* `homeDir()` is Tauri's; a browser specimen states one instead. */
const HOME = "/Users/example";

/* What the folder dialog would hand back. Long on purpose: the field must
   still be readable next to the select and Browse. */
const BROWSE_PICK: Record<string, string> = {
  codex: "/Volumes/Work/agent-homes/codex",
  claude: "/Volumes/Work/agent-homes/claude",
};

const PROJECT = { name: "HorrorGame", folder: "~/Projects/HorrorGame" };

/* Storages carry a presentation hint the frontend turns into a glyph with
   `iconNameFromHint` — the same vocabulary the Storages landing draws. */
const STORAGES = [
  { id: "vault", name: "Vault", providerLabel: "Cloudflare R2", icon: "cloud" as IconName },
  { id: "backup", name: "Backup drive", providerLabel: "Local folder", icon: "drive" as IconName },
];

const BUNDLES = [
  { id: "bnd_7f3a2c91", name: "HorrorGame", stamp: "2 days ago", match: true },
  { id: "bnd_91ce0447", name: "HorrorGame (old Mac)", stamp: "3 weeks ago", match: false },
];

/* ——— paths ———————————————————————————————————————————————————————— */

/** `expandHomePath` verbatim, against the stated home. */
function expandHome(value: string): string {
  const trimmed = value.trim();
  if (!trimmed.startsWith("~")) return trimmed;
  if (trimmed === "~") return HOME;
  if (trimmed.startsWith("~/")) return `${HOME}/${trimmed.slice(2)}`;
  return trimmed;
}

/** `isAbsoluteProfilePath` verbatim: POSIX root, drive letter, or UNC share. */
function isAbsolutePath(value: string): boolean {
  return value.startsWith("/")
    || /^[A-Za-z]:[\\/]/.test(value)
    || value.startsWith("\\\\");
}

/** The display name Add derives for a path it is about to save: the folder
 *  you pointed at, not the dot-directory found inside it. */
function nameFromFolder(path: string): string {
  const parts = path.replace(/\/+$/, "").split(/[\\/]/).filter(Boolean);
  while (parts.length > 1 && parts[parts.length - 1].startsWith(".")) parts.pop();
  return parts[parts.length - 1] ?? "profile";
}

interface PathHint {
  tone: "mute" | "ok" | "warn" | "bad";
  message: string;
  path?: string;
}

/**
 * `profilePathHint` from ProjectSetupWorkspace, keeping every state a
 * specimen can derive honestly: the two typing states, the reuse case, and
 * the two shapes the probe returns for a good folder. Same tones, same
 * sentences, same slot in the row. The two states that need a real
 * filesystem — "can't be read" (warn) and a probe error (bad) — land in this
 * same line; the tone classes are already in App.css.
 */
function pathHint(raw: string, provider: string): PathHint {
  const expanded = expandHome(raw);
  if (!expanded) {
    return { tone: "mute", message: "Type or paste a path, or Browse. ~ means your home folder." };
  }
  if (!isAbsolutePath(expanded)) {
    return { tone: "mute", message: "Paths must be absolute — start with ~/, / or a drive letter." };
  }
  const registered = PROFILES.find((profile) => expandHome(profile.path) === expanded);
  if (registered) {
    return { tone: "ok", message: `Already set up as “${registered.name}” — setup will reuse it.` };
  }
  /* The probe looks one level down when you hand it a parent rather than the
     profile directory itself. */
  const dir = DEFAULT_DIR[provider];
  const last = expanded.replace(/\/+$/, "").split("/").pop() ?? "";
  if (dir && last !== dir) {
    return { tone: "ok", message: `Found ${dir} inside — using`, path: `${expanded.replace(/\/+$/, "")}/${dir}` };
  }
  return { tone: "ok", message: `${PROVIDER_LABEL[provider]} profile ·`, path: expanded };
}

/* ——— the profile field, shared by both pages ——————————————————————— */

interface ProfileField {
  provider: string;
  profileId: string | null;
  /** Non-null exactly while the custom path field is open. */
  pending: string | null;
  choose: (next: string) => void;
  setPending: (next: string) => void;
  browse: () => void;
}

interface Resolved { name: string; path: string }

/** What one profile field resolves to — a saved home, or a valid typed path
 *  named after the folder it points at. Null while the answer is missing. */
function resolveProfile(field: ProfileField): Resolved | null {
  const saved = PROFILES.find((profile) => profile.id === field.profileId);
  if (saved) return { name: saved.name, path: expandHome(saved.path) };
  if (field.pending === null) return null;
  const expanded = expandHome(field.pending);
  if (!isAbsolutePath(expanded)) return null;
  const path = pathHint(field.pending, field.provider).path ?? expanded;
  return { name: nameFromFolder(path), path };
}

interface FieldState { profileId: string | null; pending: string | null }

/** One store for however many providers a page asks about: the Catalog page
 *  asks about one, project setup about every agent you enable. */
function useProfileFields() {
  const [state, setState] = useState<Record<string, FieldState>>({});
  const write = (provider: string, next: FieldState) =>
    setState((current) => ({ ...current, [provider]: next }));
  const field = (provider: string): ProfileField => ({
    provider,
    profileId: state[provider]?.profileId ?? null,
    pending: state[provider]?.pending ?? null,
    choose: (next) => write(provider, next === "__pending"
      // Open the field, not the folder dialog. A pasted path, or one on a
      // volume that is not mounted yet, has no way through a picker.
      ? { profileId: null, pending: "" }
      : { profileId: next || null, pending: null }),
    setPending: (next) => write(provider, { profileId: null, pending: next }),
    // Browse fills the field whatever the select was on, as the page does.
    browse: () => write(provider, {
      profileId: null,
      pending: BROWSE_PICK[provider] ?? `${HOME}/agent-home`,
    }),
  });
  const preselect = (provider: string) => write(provider, {
    profileId: PROFILES.find((profile) => profile.provider === provider)?.id ?? null,
    pending: null,
  });
  const forget = (provider: string) => setState((current) => {
    const { [provider]: _dropped, ...rest } = current;
    return rest;
  });
  return { field, preselect, forget };
}

/* ——— page state ——————————————————————————————————————————————————— */

/** Add Catalog resource: Kind, then provider, then that provider's profile. */
function useCatalog() {
  const [kindKey, setKindKey] = useState<string | null>(null);
  const [provider, setProvider] = useState<string | null>(null);
  const fields = useProfileFields();

  const kind = KINDS.find((candidate) => candidate.key === kindKey) ?? null;
  const offered = kind ? kind.providers : ALL_PROVIDERS;
  const field = provider === null ? null : fields.field(provider);
  const resolved = field === null ? null : resolveProfile(field);

  const chooseKind = (next: string) => {
    const picked = KINDS.find((candidate) => candidate.key === next) ?? null;
    setKindKey(next);
    // A provider the new shape is not registered for is dropped rather than
    // kept invalid.
    if (provider !== null && picked !== null && !picked.providers.includes(provider)) {
      fields.forget(provider);
      setProvider(null);
    }
  };
  const chooseProvider = (next: string) => {
    setProvider(next);
    fields.preselect(next);
  };

  return {
    kind, kindKey, chooseKind,
    provider, chooseProvider, offered,
    field, resolved,
    ready: kind !== null && provider !== null && resolved !== null,
  };
}

type Catalog = ReturnType<typeof useCatalog>;

/** Set up <project>: the folder is already chosen by the time the draft
 *  exists; what is left is the agents, the storage, and the repository. */
function useProject() {
  const [agents, setAgents] = useState<string[]>(["claude"]);
  const [storageId, setStorageId] = useState<string>("local");
  const [repo, setRepo] = useState<string>("new");
  const fields = useProfileFields();

  const storage = STORAGES.find((candidate) => candidate.id === storageId) ?? null;
  const agentFields = agents.map((provider) => fields.field(provider));
  const resolvedAgents = agentFields.map(resolveProfile);
  const bundle = BUNDLES.find((candidate) => candidate.id === repo) ?? null;

  const toggleAgent = (provider: string) => {
    if (!agents.includes(provider)) {
      fields.preselect(provider);
      setAgents([...agents, provider]);
      return;
    }
    // The page never lets you turn the last one off.
    if (agents.length === 1) return;
    fields.forget(provider);
    setAgents(agents.filter((candidate) => candidate !== provider));
  };
  const chooseStorage = (next: string) => {
    setStorageId(next);
    // Repository choices are storage-specific.
    setRepo("new");
  };

  return {
    agents, toggleAgent, field: fields.field,
    storageId, storage, chooseStorage,
    repo, setRepo, bundle,
    agentsReady: resolvedAgents.every((entry) => entry !== null),
    resolvedAgents,
    ready: agents.length > 0 && resolvedAgents.every((entry) => entry !== null),
  };
}

type Project = ReturnType<typeof useProject>;

/* ——— shared furniture ——————————————————————————————————————————————— */

function attentionBadge(show: boolean) {
  if (!show) return null;
  return (
    <span className="sync-setup-state attention" aria-label="Needs review" title="Needs review">
      <Icon name="alert-triangle" size={12} />
    </span>
  );
}

function Frame({ head, children, ready, cancelLabel, actionLabel, actionIcon }: {
  head: ReactNode;
  children: ReactNode;
  ready: boolean;
  cancelLabel: string;
  actionLabel: string;
  actionIcon: IconName;
}) {
  return (
    <main className="sync-main sync-project-links-page sync-project-setup-page mock-page">
      <section className="profile-links-section">
        <section className="sync-inline-new-project sync-setup-workspace" role="region" aria-label="Setup">
          <header className="sync-modal-header">
            <div><h1>{head}</h1></div>
            <button type="button" className="btn btn-ghost" aria-label="Close">
              <Icon name="x" size={12} />
            </button>
          </header>
          <div className="sync-modal-body">{children}</div>
          <footer className="sync-modal-footer sync-setup-footer">
            <div>
              <button type="button" className="btn btn-ghost">{cancelLabel}</button>
              <button type="button" className="btn btn-primary" disabled={!ready}>
                <Icon name={actionIcon} size={12} /> {actionLabel}
              </button>
            </div>
          </footer>
        </section>
      </section>
    </main>
  );
}

function Band({ title, badge, children }: { title: string; badge?: ReactNode; children: ReactNode }) {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading">
        <div><strong>{title}</strong></div>
        {badge}
      </div>
      {children}
    </section>
  );
}

function StepBand({ step, done, title, note, children }: {
  step: number;
  done: boolean;
  title: string;
  note?: string;
  children: ReactNode;
}) {
  return (
    <section className="sync-setup-section">
      <div className="sync-card-heading setup-step-heading">
        <span className={`setup-step-mark${done ? " done" : ""}`} aria-hidden="true">
          {done ? <Icon name="check" size={12} /> : step}
        </span>
        <div>
          <strong>{title}</strong>
          {note && <span>{note}</span>}
        </div>
      </div>
      {children}
    </section>
  );
}

/** One choice, whatever list draws it. */
interface Choice {
  key: string;
  label: string;
  /** The glyph the row wears in A/B/C. */
  icon: IconName;
  /** The line under the name — marks plus a sentence, or a bare identifier. */
  note: ReactNode;
  /** Marks that belong to the choice itself, drawn on the card's foot. */
  marks?: ReactNode;
  /** A right-hand stamp: the repository list's relative time. */
  meta?: ReactNode;
  /** "Git match" and friends, inside the name. */
  chip?: ReactNode;
  /** The shipped `separate` modifier — a row that is not one of a kind with
   *  the rows under it, like "New repository" above the stored ones. */
  separate?: boolean;
}

/** The shipped radio list. `marked` is the whole difference between today's
 *  row and A/C's: the choice's glyph, in a fixed slot before the copy. */
function ChoiceRows({ items, value, onPick, marked, label }: {
  items: Choice[];
  value: string | null;
  onPick: (key: string) => void;
  marked: boolean;
  label: string;
}) {
  return (
    <div className="sync-bundle-match-options" role="radiogroup" aria-label={label}>
      {items.map((option) => {
        const active = value === option.key;
        return (
          <button
            key={option.key}
            type="button"
            role="radio"
            aria-checked={active}
            className={`sync-bundle-match-option${option.separate ? " separate" : ""}${active ? " active" : ""}`}
            onClick={() => onPick(option.key)}
          >
            <span className="sync-bundle-radio"><span /></span>
            {marked && (
              <span className="setup-choice-mark" aria-hidden="true">
                <Icon name={option.icon} size={24} />
              </span>
            )}
            <span className="sync-bundle-match-copy">
              <strong>{option.label}{option.chip}</strong>
              <span className={marked ? "setup-choice-note" : undefined}>{option.note}</span>
            </span>
            {option.meta && <span className="sync-bundle-match-meta">{option.meta}</span>}
          </button>
        );
      })}
    </div>
  );
}

/** B's picker: the landing door's anatomy, made selectable. */
function ChoiceCards({ items, value, onPick, label }: {
  items: Choice[];
  value: string | null;
  onPick: (key: string) => void;
  label: string;
}) {
  return (
    <div className="setup-choice-grid" role="radiogroup" aria-label={label}>
      {items.map((option) => {
        const active = value === option.key;
        return (
          <button
            key={option.key}
            type="button"
            role="radio"
            aria-checked={active}
            className={`setup-choice-card${active ? " active" : ""}`}
            onClick={() => onPick(option.key)}
          >
            <span className="setup-choice-card-head">
              <span className="setup-choice-mark" aria-hidden="true">
                <Icon name={option.icon} size={24} />
              </span>
              <strong>{option.label}</strong>
              <span className="setup-choice-card-check" aria-hidden="true">
                {active && <Icon name="check" size={12} />}
              </span>
            </span>
            <span className="setup-choice-card-foot">{option.marks ?? option.note}</span>
          </button>
        );
      })}
    </div>
  );
}

/** A short list that was a native <select>: one pill each, wearing its own
 *  mark. The check sits in a fixed slot so picking one never reflows the row. */
function PillRow({ items, value, onPick, label }: {
  items: { key: string; label: string; mark: ReactNode }[];
  value: string | null;
  onPick: (key: string) => void;
  label: string;
}) {
  return (
    <div className="setup-pill-row" role="radiogroup" aria-label={label}>
      {items.map((item) => {
        const active = value === item.key;
        return (
          <button
            key={item.key}
            type="button"
            role="radio"
            aria-checked={active}
            className={`setup-pill${active ? " active" : ""}`}
            onClick={() => onPick(item.key)}
          >
            {item.mark}
            <span className="setup-pill-label">{item.label}</span>
            <span className="setup-pill-check" aria-hidden="true">
              {active && <Icon name="check" size={12} />}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/**
 * The profile row, the project setup page's own: a select of saved homes with
 * "Custom path…" at the end, a path field that opens when it is chosen, and
 * Browse. A home you can reach with a picker is one case; a pasted path, a
 * volume that is not mounted yet, or a folder the dialog will not show you
 * are the others, so the field is the primary control and Browse fills it.
 *
 * The verdict on what you typed lands under the field in the shipped
 * `.sync-setup-path-hint`, on the shipped tones. The row is wrapped in
 * `.sync-agent-profile-list` — the project setup page's own wrapper — which
 * supplies the container the row's narrow-width fallback queries; without it
 * the path field never takes its own line on a short window.
 *
 * `resolved` adds one line for a *saved* home too, so a page that shows the
 * folder for a typed path shows it for a picked one. Same element, same slot.
 */
function ProfileRow({ field, resolved }: { field: ProfileField; resolved: boolean }) {
  const label = PROVIDER_LABEL[field.provider] ?? field.provider;
  const compatible = PROFILES.filter((profile) => profile.provider === field.provider);
  const saved = PROFILES.find((profile) => profile.id === field.profileId) ?? null;
  const pending = field.pending;
  const hint = pending === null ? null : pathHint(pending, field.provider);
  const hintId = `setup-path-hint-${field.provider}`;
  return (
    <div className="sync-agent-profile-list">
      <div className={`sync-agent-profile-row${pending === null ? "" : " has-custom-path"}`}>
        {/* A native select cannot hold an <svg>, so the app's chevron rides a
            pseudo-element on this shell. */}
        <span className="sync-select-shell">
          <select
            aria-label={`${label} profile`}
            aria-describedby={pending === null ? undefined : hintId}
            value={pending === null ? field.profileId ?? "" : "__pending"}
            onChange={(event) => field.choose(event.target.value)}
          >
            <option value="">Choose profile</option>
            {compatible.map((profile) => (
              <option key={profile.id} value={profile.id}>{profile.name}</option>
            ))}
            <option value="__pending">Custom path…</option>
          </select>
        </span>
        {pending !== null && (
          <input
            value={pending}
            aria-label={`${label} custom profile path`}
            aria-invalid={hint?.tone === "bad" || undefined}
            aria-describedby={hintId}
            placeholder={`${label} profile path`}
            spellCheck={false}
            autoFocus
            onChange={(event) => field.setPending(event.target.value)}
          />
        )}
        <button type="button" className="btn" onClick={() => field.browse()}>
          <Icon name="folder" size={12} /> Browse
        </button>
        {hint && (
          /* Polite, not an alert: it changes while the field still holds
             focus, where a described-by change alone would go unannounced. */
          <p id={hintId} className={`sync-setup-path-hint ${hint.tone}`} aria-live="polite">
            {hint.message}
            {hint.path && <code>{hint.path}</code>}
          </p>
        )}
        {hint === null && resolved && saved && (
          <p className="sync-setup-path-hint ok spans-row">
            {label} profile ·<code>{expandHome(saved.path)}</code>
          </p>
        )}
      </div>
    </div>
  );
}

/** C's last band on both pages: the object the button is about to make,
 *  drawn the way the list that will hold it draws its rows. Dashed and barred
 *  until every answer is in — the landing doors' own "nothing here yet". */
function ResultCard({ ready, head, pendingHead, icon, title, details }: {
  ready: boolean;
  head: string;
  pendingHead: string;
  icon: IconName | null;
  title: string | null;
  details: ReactNode[];
}) {
  return (
    <div className={`setup-result${ready ? "" : " pending"}`}>
      <div className="setup-result-head">
        <Icon name={ready ? "check-circle" : "clock"} size={12} />
        {ready ? head : pendingHead}
      </div>
      <div className="setup-result-row">
        <span className="setup-choice-mark" aria-hidden="true">
          <Icon name={icon ?? "file"} size={24} />
        </span>
        <span className="setup-result-copy">
          {title ? <strong>{title}</strong> : <span className="setup-result-bar wide" />}
          {details.length > 0
            ? details.map((detail, index) => <span key={index}>{detail}</span>)
            : <span className="setup-result-bar" />}
        </span>
      </div>
    </div>
  );
}

/* ——— Catalog: the four panels —————————————————————————————————————— */

function kindChoices(marked: boolean): Choice[] {
  return KINDS.map((kind) => {
    const names = kind.providers.map((provider) => PROVIDER_LABEL[provider]);
    const sentence = names.length === 1 ? `${names[0]} only` : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
    const marks = kind.providers.map((provider) => (
      <ProviderMark key={provider} provider={provider} displayName={PROVIDER_LABEL[provider]} size={12} />
    ));
    return {
      key: kind.key,
      label: kind.label,
      icon: kind.icon,
      note: marked ? <>{marks}{sentence}</> : sentence,
      marks: <>{marks}{sentence}</>,
      separate: true,
    };
  });
}

function providerPills(offered: string[]) {
  return offered.map((provider) => ({
    key: provider,
    label: PROVIDER_LABEL[provider],
    mark: <ProviderMark provider={provider} displayName={PROVIDER_LABEL[provider]} size={12} />,
  }));
}

function CatalogFrame({ page, children }: { page: Catalog; children: ReactNode }) {
  return (
    <Frame
      head="Add Catalog resource"
      ready={page.ready}
      cancelLabel="Cancel"
      actionLabel="Add resource"
      actionIcon="plus"
    >
      {children}
    </Frame>
  );
}

function CatalogToday() {
  const page = useCatalog();
  return (
    <CatalogFrame page={page}>
      <Band title="Kind" badge={attentionBadge(page.kind === null)}>
        <ChoiceRows items={kindChoices(false)} value={page.kindKey} onPick={page.chooseKind} marked={false} label="Resource kind" />
      </Band>
      <Band title="Agent profile" badge={attentionBadge(page.kind !== null && page.resolved === null)}>
        <div className="sync-profile-select-row">
          <span className="sync-select-shell">
            <select
              aria-label="Provider"
              value={page.provider ?? ""}
              onChange={(event) => (event.target.value ? page.chooseProvider(event.target.value) : undefined)}
            >
              <option value="">Choose provider</option>
              {page.offered.map((provider) => (
                <option key={provider} value={provider}>{PROVIDER_LABEL[provider]}</option>
              ))}
            </select>
          </span>
        </div>
        {page.field && <ProfileRow field={page.field} resolved={false} />}
        {page.kind !== null && page.provider === null && (
          <small className="sync-setup-hint">Each Catalog resource tracks one kind from one agent profile.</small>
        )}
      </Band>
    </CatalogFrame>
  );
}

function CatalogA() {
  const page = useCatalog();
  return (
    <CatalogFrame page={page}>
      <Band title="Kind" badge={attentionBadge(page.kind === null)}>
        <ChoiceRows items={kindChoices(true)} value={page.kindKey} onPick={page.chooseKind} marked label="Resource kind" />
      </Band>
      <Band title="Agent profile" badge={attentionBadge(page.kind !== null && page.resolved === null)}>
        <PillRow items={providerPills(page.offered)} value={page.provider} onPick={page.chooseProvider} label="Provider" />
        {page.field && <ProfileRow field={page.field} resolved />}
        {page.kind !== null && page.provider === null && (
          <small className="sync-setup-hint">Each Catalog resource tracks one kind from one agent profile.</small>
        )}
      </Band>
    </CatalogFrame>
  );
}

function CatalogB() {
  const page = useCatalog();
  return (
    <CatalogFrame page={page}>
      <Band title="Kind" badge={attentionBadge(page.kind === null)}>
        <ChoiceCards items={kindChoices(true)} value={page.kindKey} onPick={page.chooseKind} label="Resource kind" />
      </Band>
      <Band title="Agent profile" badge={attentionBadge(page.kind !== null && page.resolved === null)}>
        <PillRow items={providerPills(page.offered)} value={page.provider} onPick={page.chooseProvider} label="Provider" />
        {page.field && <ProfileRow field={page.field} resolved />}
      </Band>
    </CatalogFrame>
  );
}

function CatalogC() {
  const page = useCatalog();
  return (
    <CatalogFrame page={page}>
      <StepBand step={1} done={page.kind !== null} title="Kind">
        <ChoiceRows items={kindChoices(true)} value={page.kindKey} onPick={page.chooseKind} marked label="Resource kind" />
      </StepBand>
      <StepBand step={2} done={page.provider !== null} title="Agent">
        <PillRow items={providerPills(page.offered)} value={page.provider} onPick={page.chooseProvider} label="Provider" />
      </StepBand>
      <StepBand
        step={3}
        done={page.resolved !== null}
        title="Profile home"
        note={page.provider === null ? "Choose an agent first — each resource tracks one kind from one profile." : undefined}
      >
        {page.field && <ProfileRow field={page.field} resolved />}
      </StepBand>
      <section className="sync-setup-section">
        <ResultCard
          ready={page.ready}
          head="Adds to Catalog"
          pendingHead="Will be added to Catalog"
          icon={page.kind?.icon ?? null}
          title={page.kind?.label ?? null}
          details={page.resolved && page.provider ? [
            <>
              <ProviderMark provider={page.provider} displayName={PROVIDER_LABEL[page.provider]} size={12} />
              {page.resolved.name} · {page.resolved.path}
            </>,
          ] : []}
        />
      </section>
    </CatalogFrame>
  );
}

/* ——— project setup: the four panels ———————————————————————————————— */

function storagePills() {
  return [
    { key: "local", label: "Local only", mark: <Icon name="computer" size={12} /> },
    ...STORAGES.map((storage) => ({
      key: storage.id,
      label: storage.name,
      mark: <Icon name={storage.icon} size={12} />,
    })),
  ];
}

function storageChoices(): Choice[] {
  return [
    {
      key: "local",
      label: "Local only",
      icon: "computer",
      note: "Nothing leaves this Mac",
      marks: "Nothing leaves this Mac",
    },
    ...STORAGES.map((storage) => ({
      key: storage.id,
      label: storage.name,
      icon: storage.icon,
      note: storage.providerLabel,
      marks: storage.providerLabel,
    })),
  ];
}

function repositoryChoices(page: Project): Choice[] {
  const storageName = page.storage?.name ?? "the selected storage";
  return [
    {
      key: "new",
      label: "New repository",
      icon: "git-fork",
      note: `Creates “${PROJECT.name}” in ${storageName} — machines joining later will find it there`,
      separate: true,
    },
    ...BUNDLES.map((bundle) => ({
      key: bundle.id,
      label: bundle.name,
      icon: "git-folder" as IconName,
      note: bundle.id,
      meta: bundle.stamp,
      chip: bundle.match ? <small className="sync-bundle-recommended">Git match</small> : undefined,
    })),
  ];
}

/** The Agent band: one card per registered agent, a checkbox to use it here,
 *  and that agent's profile row inside. `marked` puts the agent's own mark on
 *  the card's name line — the counterpart of the Catalog page's pills. */
function AgentCards({ page, marked }: { page: Project; marked: boolean }) {
  return (
    <div className="sync-provider-cards" role="group" aria-label="Agents used by this project">
      {ALL_PROVIDERS.map((provider) => {
        const enabled = page.agents.includes(provider);
        return (
          <div key={provider} className={`sync-provider-card${enabled ? " enabled" : ""}`}>
            <label className={`sync-provider-toggle${marked ? " setup-marked-toggle" : ""}`}>
              <input
                type="checkbox"
                checked={enabled}
                disabled={enabled && page.agents.length === 1}
                aria-label={`Use ${PROVIDER_LABEL[provider]} in this project`}
                onChange={() => page.toggleAgent(provider)}
              />
              {marked && <ProviderMark provider={provider} displayName={PROVIDER_LABEL[provider]} size={12} />}
              <strong>{PROVIDER_LABEL[provider]}</strong>
              <span>{PROVIDER_LABEL[provider]} profile</span>
            </label>
            {enabled
              ? <ProfileRow field={page.field(provider)} resolved={marked} />
              : <small>Not used by this project on this machine.</small>}
          </div>
        );
      })}
    </div>
  );
}

function ProjectFields({ marked }: { marked: boolean }) {
  return (
    <div className="sync-field-grid sync-setup-project-fields">
      <label>
        <span>Name</span>
        <input defaultValue={PROJECT.name} />
      </label>
      <label>
        <span>Folder</span>
        <div className="sync-profile-select-row">
          <input value={PROJECT.folder} readOnly title={expandHome(PROJECT.folder)} />
          <button type="button" className="btn">
            {marked && <Icon name="folder" size={12} />} Change…
          </button>
        </div>
      </label>
    </div>
  );
}

function ProjectFrame({ page, children }: { page: Project; children: ReactNode }) {
  return (
    <Frame
      head={`Set up ${PROJECT.name}`}
      ready={page.ready}
      cancelLabel="Discard"
      actionLabel={page.storage ? "Finish" : "Finish locally"}
      actionIcon="check"
    >
      {children}
    </Frame>
  );
}

function ProjectToday() {
  const page = useProject();
  return (
    <ProjectFrame page={page}>
      <Band title="Project"><ProjectFields marked={false} /></Band>
      <Band title="Agent" badge={attentionBadge(!page.agentsReady)}>
        <AgentCards page={page} marked={false} />
      </Band>
      <Band title="Storage">
        <div className="sync-profile-select-row">
          <span className="sync-select-shell">
            <select aria-label="Storage" value={page.storageId} onChange={(event) => page.chooseStorage(event.target.value)}>
              <option value="local">Local only</option>
              {STORAGES.map((storage) => (
                <option key={storage.id} value={storage.id}>{storage.name} · {storage.providerLabel}</option>
              ))}
            </select>
          </span>
        </div>
      </Band>
      {page.storage && (
        <Band title="Repository">
          <ChoiceRows items={repositoryChoices(page)} value={page.repo} onPick={page.setRepo} marked={false} label="Repo identity for this project" />
        </Band>
      )}
    </ProjectFrame>
  );
}

function ProjectA() {
  const page = useProject();
  return (
    <ProjectFrame page={page}>
      <Band title="Project"><ProjectFields marked /></Band>
      <Band title="Agent" badge={attentionBadge(!page.agentsReady)}>
        <AgentCards page={page} marked />
      </Band>
      <Band title="Storage">
        <PillRow items={storagePills()} value={page.storageId} onPick={page.chooseStorage} label="Storage" />
      </Band>
      {page.storage && (
        <Band title="Repository">
          <ChoiceRows items={repositoryChoices(page)} value={page.repo} onPick={page.setRepo} marked label="Repo identity for this project" />
        </Band>
      )}
    </ProjectFrame>
  );
}

function ProjectB() {
  const page = useProject();
  return (
    <ProjectFrame page={page}>
      <Band title="Project"><ProjectFields marked /></Band>
      <Band title="Agent" badge={attentionBadge(!page.agentsReady)}>
        <AgentCards page={page} marked />
      </Band>
      <Band title="Storage">
        <ChoiceCards items={storageChoices()} value={page.storageId} onPick={page.chooseStorage} label="Storage" />
      </Band>
      {page.storage && (
        <Band title="Repository">
          <ChoiceRows items={repositoryChoices(page)} value={page.repo} onPick={page.setRepo} marked label="Repo identity for this project" />
        </Band>
      )}
    </ProjectFrame>
  );
}

function ProjectC() {
  const page = useProject();
  const repoLabel = page.bundle?.name ?? "New repository";
  return (
    <ProjectFrame page={page}>
      <StepBand step={1} done title="Project"><ProjectFields marked /></StepBand>
      <StepBand step={2} done={page.agentsReady} title="Agent">
        <AgentCards page={page} marked />
      </StepBand>
      <StepBand step={3} done title="Storage">
        <PillRow items={storagePills()} value={page.storageId} onPick={page.chooseStorage} label="Storage" />
      </StepBand>
      {page.storage && (
        <StepBand step={4} done title="Repository">
          <ChoiceRows items={repositoryChoices(page)} value={page.repo} onPick={page.setRepo} marked label="Repo identity for this project" />
        </StepBand>
      )}
      <section className="sync-setup-section">
        <ResultCard
          ready={page.ready}
          head={page.storage ? "Finish creates" : "Finish keeps this local"}
          pendingHead="Not ready to finish"
          icon="git-folder"
          title={PROJECT.name}
          details={page.ready ? [
            expandHome(PROJECT.folder),
            <>
              {page.agents.map((provider) => (
                <ProviderMark key={provider} provider={provider} displayName={PROVIDER_LABEL[provider]} size={12} />
              ))}
              {page.agents.map((provider) => PROVIDER_LABEL[provider]).join(", ")}
              {page.storage
                ? <> · <Icon name={page.storage.icon} size={12} /> {page.storage.name} · {repoLabel}</>
                : <> · <Icon name="computer" size={12} /> Local only</>}
            </>,
          ] : []}
        />
      </section>
    </ProjectFrame>
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

const NOTES = {
  today: <>
    As it ships. Every choice on both pages is made in words: bullet rows whose only
    difference is the label, a checkbox line with the agent's name spelled out, and two
    native dropdowns, which cannot hold a mark at all. Coming from a landing that drew you
    a lightbulb or a storage's cloud, the picture stops exactly where the choice is
    confirmed. The only glyph on either page is the warning triangle. The profile row is
    the same on both — pick a saved home, or choose “Custom path…” and type one; try{" "}
    <code>~/work/.claude</code>, <code>~/agents</code> and <code>agents</code> for the
    three verdicts.
  </>,
  a: <>
    The smallest change that closes the gap, and it lands on both pages at once. Same
    bands, same controls, same 44px rows: every choice wears its glyph in a fixed slot so
    the names stay on one column, agents wear their own mark on the card that enables
    them, and the two dropdowns that hold a short list become pills — a provider or a
    storage was never a menu's worth of choice. Repository rows take the git glyphs.
    Nothing moves; the pages just say in art what they were already saying in words.
  </>,
  b: <>
    The icons become the picker for the one list on each page that deserves a picture —
    Catalog Kind, project Storage — as a card grid in the landing door's anatomy: glyph,
    name in the display face, the provider or bucket on the foot line. Click a landing
    door and setup opens onto that same object, at the same size, with that card filled.
    It costs the tallest band of the three; on project setup it pushes Repository below
    the fold, which is the trade to weigh.
  </>,
  c: <>
    Numbered bands instead of plain ones. Each marker holds its step number until the step
    is answered and then closes into a check, so the warning triangle is retired —
    progress is stated once, in the same place, on both pages. Under the steps, a result
    card names exactly what the button will make: the Catalog row, or the project with its
    folder, agents, storage and repository. It is dashed and barred until the answers are
    in. Most furniture of the three, and the only one that shows you the outcome before
    you commit to it.
  </>,
};

function Harness() {
  const [dark, setDark] = useState(() => document.documentElement.getAttribute("data-theme") === "dark");
  const [page, setPage] = useState<"catalog" | "project">("catalog");
  const catalog = page === "catalog";
  return (
    <div className="app mockup-root">
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>Setup pages, with icons</h1>
            <p>
              Both setup pages ask the reader to pick things the rest of the app already
              draws — Catalog kinds, storages, agents — and both answer in plain text.
              Three directions below, ordered by how much work the icons do: identity only,
              the icons as the picker, then identity plus state plus the outcome. Each
              direction is applied to both pages from one set of parts, so whatever is
              picked lands on both at once. Every panel keeps its own state.
            </p>
            <div className="mockup-switch" role="radiogroup" aria-label="Page">
              <button
                type="button"
                role="radio"
                aria-checked={catalog}
                className={`btn${catalog ? " btn-primary" : ""}`}
                onClick={() => setPage("catalog")}
              >
                <Icon name="catalog" size={12} /> Add Catalog resource
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={!catalog}
                className={`btn${catalog ? "" : " btn-primary"}`}
                onClick={() => setPage("project")}
              >
                <Icon name="git-folder" size={12} /> Set up a project
              </button>
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

        <Panel title="Today — no glyph on any choice" note={NOTES.today}>
          {catalog ? <CatalogToday /> : <ProjectToday />}
        </Panel>
        <Panel title="A · Marked rows — identity only" note={NOTES.a}>
          {catalog ? <CatalogA /> : <ProjectA />}
        </Panel>
        <Panel title="B · Cards — the icons are the picker" note={NOTES.b}>
          {catalog ? <CatalogB /> : <ProjectB />}
        </Panel>
        <Panel title="C · Steps and result — identity, state, outcome" note={NOTES.c}>
          {catalog ? <CatalogC /> : <ProjectC />}
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
.mockup-switch { display: flex; gap: 8px; margin-top: 14px; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-display); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > p { max-width: 88ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-panel > p code { font-family: var(--font-mono); font-size: var(--type-xs); }
.mockup-stage { border: 1px solid var(--border); border-radius: 6px; background: var(--surface-card); overflow: hidden; }
/* The page's own top padding stands in for the window chrome the app draws. */
.mock-page { min-height: 0; padding: 14px 0 18px; overflow: visible; }
body { background: var(--app-shell); }
`;

/* Everything a cutover would lift into App.css. The variants reuse
   .sync-setup-section, .sync-bundle-match-*, .sync-provider-card* and
   .sync-agent-profile-row as they are; only `.spans-row` touches a shipped
   rule, and only by adding a modifier. */
const PROPOSED_CSS = `
/* What the picked directions landed in App.css is gone from here: the choice
   glyph, the pill row, the marked toggle, the card grid and the row-spanning
   path hint are all shipped rules now, so the panels below render the app's
   own CSS. What remains is the two directions nobody picked. */

/* A · the line under a choice's name when it carries marks. */
.setup-choice-note {
  overflow: visible;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}

.setup-choice-note > svg,
.setup-choice-note > .sync-provider-mark {
  flex: 0 0 auto;
  color: var(--text-2);
}

/* ——— C: step markers and the result card ——— */
.sync-setup-section > .sync-card-heading.setup-step-heading {
  align-items: center;
  justify-content: flex-start;
  gap: 11px;
}

.setup-step-heading > div { min-width: 0; flex: 1; }

.setup-step-mark {
  width: 22px;
  height: 22px;
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  border: 2px solid var(--border-mid);
  border-radius: 50%;
  color: var(--text-3);
  background: var(--surface-subtle);
  font-family: var(--font-mono);
  font-size: var(--type-2xs);
  font-variant-numeric: tabular-nums;
}

.setup-step-mark.done {
  border-color: var(--text-0);
  color: var(--text-0);
  background: var(--bg-hover);
}

.setup-result {
  max-width: var(--sync-setup-column);
  overflow: hidden;
  border: 2px solid var(--border-mid);
  border-radius: var(--retro-panel-radius);
  background: var(--surface-row);
}

.setup-result.pending {
  border-style: dashed;
  background: transparent;
}

.setup-result-head {
  min-height: 32px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid var(--border-mid);
  background: var(--surface-subtle);
  color: var(--text-2);
  font-family: var(--font-display);
  font-size: var(--type-2xs);
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

.setup-result.pending .setup-result-head {
  border-bottom-style: dashed;
  background: transparent;
  color: var(--text-3);
}

.setup-result.pending .setup-choice-mark { color: var(--text-3); }

.setup-result-row {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 11px 12px;
}

.setup-result-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.setup-result-copy > strong {
  color: var(--text-0);
  font-family: var(--font-display);
  font-size: var(--type-lg);
  font-weight: var(--weight-bold);
}

.setup-result-copy > span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--text-3);
  font-family: var(--font-mono);
  font-size: var(--type-2xs);
}

.setup-result-copy > span > svg,
.setup-result-copy > span > .sync-provider-mark { flex: 0 0 auto; }

.setup-result-bar {
  display: block;
  width: 38%;
  min-width: 92px;
  height: 8px;
  border-radius: var(--r-sm);
  background: var(--chip-bg);
}

.setup-result-bar.wide { width: 54%; min-width: 132px; }
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

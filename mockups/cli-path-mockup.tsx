import { StrictMode, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import SettingsPage from "../src/components/project-sync/SettingsPage";
import SettingsSectionIcon from "../src/components/project-sync/SettingsSectionIcon";
import { projectSyncApi } from "../src/components/project-sync/api";
import Icon from "../src/components/Icons";
import type { AppUpdates } from "../src/components/AppUpdater";
import { t, uiLanguage } from "../src/i18n";
import type { AppSettings, CliSelectionSource, ProviderCliStatus, ProviderDescriptor } from "../src/types";
import "../src/App.css";

/**
 * Mockup: Settings › Agent CLIs, with a path the user can type.
 *
 * The first frame is the real `SettingsPage` on a stand-in backend — since
 * 2026-10-08 it renders A, the direction the owner picked. Every other frame
 * is the CLI card drawn with the app's own classes (the settings row, the
 * project folder field, the path hint tones); mock-only rules are fenced in
 * the html. A typed path saves on Return or Browse only, never on blur — a
 * half-typed `/Users` would pass the save's check. The stand-in answers the way the Rust does today: a save checks
 * only that the path is absolute, then discovery says whether it runs.
 *
 *   A  field, detected path as placeholder        no backend change
 *   B  Automatic / Custom path… pop-up             no backend change
 *   C  pop-up of found installs + Custom path…     lists every install
 *   D  path as a value + Change…                    no backend change
 *   E  editable combo box of found installs        lists every install
 *   F  field checked before it saves               commands take a path
 *
 * /mockups/cli-path-mockup.html
 * (`?state=auto|auto-standard|detecting|missing|custom|unavailable|typed|refused|tested|test-failed
 *   &width=760|560&theme=light|dark&ui=zh-CN|en&only=before|A|…|F`).
 * The state knob drives the Codex row; Claude stays found on the shell PATH.
 */

/* ——— knobs ——— */

type Dir = "before" | "A" | "B" | "C" | "D" | "E" | "F";
type State =
  | "auto" | "auto-standard" | "detecting" | "missing" | "custom"
  | "unavailable" | "typed" | "refused" | "tested" | "test-failed";
interface Knobs {
  state: State;
  width: "760" | "560";
  theme: "light" | "dark";
  ui: "zh-CN" | "en";
  only: Dir | "all";
}
const DIRS: readonly Dir[] = ["before", "A", "B", "C", "D", "E", "F"];
const STATES: readonly State[] = [
  "auto", "auto-standard", "detecting", "missing", "custom", "unavailable", "typed", "refused", "tested", "test-failed",
];
const STATE_LABEL: Record<State, string> = {
  auto: "Found · shell PATH",
  "auto-standard": "Found · install folder",
  detecting: "Detecting",
  missing: "Not found",
  custom: "Custom, works",
  unavailable: "Custom, gone",
  typed: "Typed, not saved",
  refused: "Typed, refused",
  tested: "Tested",
  "test-failed": "Test failed",
};

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    state: pick("state", STATES, "auto"),
    width: pick("width", ["760", "560"] as const, "760"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
    ui: pick("ui", ["zh-CN", "en"] as const, "zh-CN"),
    only: pick("only", ["all", ...DIRS] as const, "all"),
  };
}

const DIR_TITLE: Record<Dir, string> = {
  before: "Settings page (A, implemented)",
  A: "A · Field, detected path as placeholder",
  B: "B · Automatic / Custom path… pop-up",
  C: "C · Pop-up of found installs",
  D: "D · Path as a value + Change…",
  E: "E · Editable combo box",
  F: "F · Checked before it saves",
};
/* One "seen in" line, one against, and what the backend would need. */
const DIR_SEEN: Record<Dir, { seen: Array<[string, string]>; against: string | null; backend: string | null }> = {
  before: { seen: [], against: null, backend: null },
  A: {
    seen: [["JetBrains Git path", "https://github.com/JetBrains/intellij-community/blob/master/platform/vcs-impl/src/com/intellij/util/ui/VcsExecutablePathSelector.java"]],
    against: "Placeholder reads as filled (NN/g)",
    backend: null,
  },
  B: {
    seen: [
      ["GitHub Desktop custom editor", "https://docs.github.com/en/desktop/configuring-and-customizing-github-desktop/configuring-a-default-editor-in-github-desktop"],
      ["HIG pop-up Custom item", "https://developer.apple.com/design/human-interface-guidelines/pop-up-buttons"],
      ["Mallard project setup agent row", "#"],
    ],
    against: "One more click to type",
    backend: null,
  },
  C: {
    seen: [
      ["Xcode Command Line Tools", "https://developer.apple.com/documentation/xcode/configuring-command-line-tools-settings"],
      ["VS Code Python interpreter picker", "https://code.visualstudio.com/docs/python/environments"],
    ],
    against: "Long paths in a menu",
    backend: "lists every install found",
  },
  D: {
    seen: [
      ["JetBrains Node runtime", "https://www.jetbrains.com/help/idea/developing-node-js-applications.html"],
      ["PatternFly inline edit", "https://www.patternfly.org/components/inline-edit/design-guidelines"],
    ],
    against: "Field hidden behind a click",
    backend: null,
  },
  E: {
    seen: [
      ["HIG combo boxes", "https://developer.apple.com/design/human-interface-guidelines/combo-boxes"],
      ["ARIA APG combobox", "https://www.w3.org/WAI/ARIA/apg/patterns/combobox/"],
    ],
    against: "List wider than field (HIG)",
    backend: "lists every install found",
  },
  F: {
    seen: [
      ["GitHub Desktop invalid-path line", "https://github.com/desktop/desktop/blob/development/app/src/ui/preferences/custom-integration-form.tsx"],
      ["Mallard project folder field", "#"],
    ],
    against: "Extra Save step",
    backend: "inspect and test take a path",
  },
};

/* ——— words the frames add (the rest are the app's own catalog) ——— */

const COPY = {
  en: {
    automatic: "Automatic",
    custom: "Custom",
    customPath: "Custom path…",
    src_inherited_path: "Mallard's PATH",
    src_shell_path: "shell PATH",
    src_standard_location: "install folder",
    src_selected: "custom",
    browse: "Browse…",
    change: "Change…",
    testing: "Testing…",
    enterToSave: "Press Return to save.",
    autoPlaceholder: "Auto-detected: {{path}}",
    nonePlaceholder: "Not found. Type the path to {{name}}.",
    pathPlaceholder: "/path/to/{{program}}",
    pathLabel: "{{name}} CLI path",
    executable: "Executable",
    browseItem: "Browse…",
    autoItem: "Automatic ({{where}})",
    showInstalls: "Show installs found",
  },
  "zh-CN": {
    automatic: "自动",
    custom: "自定义",
    customPath: "自定义路径…",
    src_inherited_path: "Mallard 的 PATH",
    src_shell_path: "Shell 的 PATH",
    src_standard_location: "常用安装目录",
    src_selected: "自定义",
    browse: "浏览…",
    change: "更改…",
    testing: "正在测试…",
    enterToSave: "按 Return 保存。",
    autoPlaceholder: "自动检测：{{path}}",
    nonePlaceholder: "未找到。输入 {{name}} 的路径。",
    pathPlaceholder: "/路径/{{program}}",
    pathLabel: "{{name}} CLI 路径",
    executable: "可执行文件",
    browseItem: "浏览…",
    autoItem: "自动（{{where}}）",
    showInstalls: "显示找到的安装",
  },
} as const;
type CopyKey = keyof typeof COPY.en;
const L = (key: CopyKey, vars: Record<string, string> = {}) =>
  Object.entries(vars).reduce<string>(
    (text, [name, value]) => text.replace(`{{${name}}}`, value),
    COPY[uiLanguage() === "zh-CN" ? "zh-CN" : "en"][key],
  );

/* ——— the stand-in machine ——— */

type Provider = "codex" | "claude";
const NAME: Record<Provider, string> = { codex: "Codex", claude: "Claude" };
const HOME = "/Users/you";
const NVM = `${HOME}/.nvm/versions/node/v22.22.0/bin`;
interface Install { path: string; source: CliSelectionSource }
const INSTALLS: Record<Provider, Install[]> = {
  codex: [
    { path: `${NVM}/codex`, source: "shell_path" },
    { path: "/opt/homebrew/bin/codex", source: "standard_location" },
  ],
  claude: [
    { path: `${NVM}/claude`, source: "shell_path" },
    { path: `${HOME}/.local/bin/claude`, source: "standard_location" },
  ],
};
const WRAPPER = `${HOME}/bin/codex-wrapper`;
const GONE = `${HOME}/.nvm/versions/node/v20.19.0/bin/codex`;
/** What `--version` reports per executable; null runs but reports nothing. */
const VERSIONS: Record<string, string | null> = {
  [`${NVM}/codex`]: "0.160.0",
  "/opt/homebrew/bin/codex": "0.156.0",
  [`${NVM}/claude`]: "2.1.30",
  [`${HOME}/.local/bin/claude`]: "2.1.28",
  [WRAPPER]: null,
};
const FOLDERS = new Set([NVM, "/opt/homebrew/bin", `${HOME}/.local/bin`, `${HOME}/bin`, HOME, "/Users", "/opt", "/opt/homebrew"]);
const BROWSED: Record<Provider, string> = { codex: "/opt/homebrew/bin/codex", claude: `${HOME}/.local/bin/claude` };

/* The backend's own sentences, English in every language (owner rule). */
const said = {
  notFound: (p: Provider) => `${NAME[p]} CLI not found. Install ${NAME[p]}, or choose its executable in Settings.`,
  unavailable: (p: Provider) => `Selected ${NAME[p]} executable is unavailable. Choose it again in Settings.`,
  notAbsolute: (p: Provider) => `${NAME[p]} CLI path must be absolute`,
  noVersion: (p: Provider) => `${NAME[p]} did not report a version.`,
};

const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

function installsFound(provider: Provider, state: State): Install[] {
  if (provider === "codex" && (state === "missing" || state === "detecting")) return [];
  if (provider === "codex" && state === "auto-standard") return INSTALLS.codex.slice(1);
  return INSTALLS[provider];
}

/** inspect_provider_cli with nothing saved: the first install discovery meets. */
function autoStatus(provider: Provider, state: State): ProviderCliStatus | null {
  if (provider === "codex" && state === "detecting") return null;
  const first = installsFound(provider, state)[0];
  if (!first) return { provider, selected_path: null, launcher: null, executable: null, source: null, error: said.notFound(provider) };
  return { provider, selected_path: null, launcher: first.path, executable: first.path, source: first.source, error: null };
}

/** A saved path is used as is or not at all: no fallback (discover_launcher). */
function resolveFake(provider: Provider, saved: string | null, state: State): ProviderCliStatus | null {
  if (!saved) return autoStatus(provider, state);
  const runs = saved in VERSIONS;
  return {
    provider,
    selected_path: saved,
    launcher: runs ? saved : null,
    executable: runs ? saved : null,
    source: runs ? "selected" : null,
    error: runs ? null : said.unavailable(provider),
  };
}

/** validate_executable's reasons, for F's check before saving. */
function checkPath(path: string): { ok: true } | { ok: false; reason: string } {
  if (!path.startsWith("/")) return { ok: false, reason: "the path is not absolute" };
  const trimmed = path.replace(/\/+$/, "");
  if (FOLDERS.has(trimmed)) return { ok: false, reason: "the path is not a regular file" };
  if (!(trimmed in VERSIONS)) return { ok: false, reason: "the file does not exist" };
  return { ok: true };
}

const initialSaved = (provider: Provider, state: State): string | null =>
  provider !== "codex" ? null
    : state === "custom" ? BROWSED.codex
      : state === "unavailable" ? GONE
        : state === "test-failed" ? WRAPPER
          : null;

interface CliTest { kind: "version" | "failed"; text: string }
const initialTest = (provider: Provider, state: State): CliTest | null =>
  provider !== "codex" ? null
    : state === "tested" ? { kind: "version", text: "0.160.0" }
      : state === "test-failed" ? { kind: "failed", text: said.noVersion("codex") }
        : null;

const initialDraft = (provider: Provider, state: State): string | null =>
  provider !== "codex" ? null : state === "typed" ? BROWSED.codex : state === "refused" ? "codex" : null;

/** One row's backend: the saved path, what discovery answers, the last Test. */
function useCli(provider: Provider, state: State) {
  const [saved, setSaved] = useState(() => initialSaved(provider, state));
  const [test, setTest] = useState<CliTest | null>(() => initialTest(provider, state));
  /** The path the last Test ran; a save of that same path keeps its answer. */
  const [testedPath, setTestedPath] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [refusal, setRefusal] = useState<string | null>(() =>
    provider === "codex" && state === "refused" ? said.notAbsolute("codex") : null);
  const [receipt, setReceipt] = useState(false);
  const status = resolveFake(provider, saved, state);
  /** applyTuning: the save refuses only a relative path; anything else is kept. */
  const commit = (raw: string): boolean => {
    const path = raw.trim();
    if (path === (saved ?? "")) {
      setRefusal(null);
      return true;
    }
    if (path && !path.startsWith("/")) {
      setRefusal(said.notAbsolute(provider));
      return false;
    }
    setRefusal(null);
    setSaved(path || null);
    if (path !== testedPath) setTest(null);
    setReceipt(true);
    window.setTimeout(() => setReceipt(false), 1600);
    return true;
  };
  /** test_provider_cli; F passes the unsaved path. */
  const runTest = async (path?: string) => {
    setTesting(true);
    await wait(350);
    const target = path ?? status?.executable ?? null;
    const version = target ? VERSIONS[target] ?? null : null;
    setTestedPath(target);
    setTest(version ? { kind: "version", text: version } : { kind: "failed", text: said.noVersion(provider) });
    setTesting(false);
  };
  return {
    provider,
    name: NAME[provider],
    program: provider,
    saved,
    status,
    test,
    testing,
    refusal,
    receipt,
    installs: installsFound(provider, state),
    /** What automatic would run, whatever is saved. */
    autoFound: autoStatus(provider, state),
    draft0: initialDraft(provider, state),
    commit,
    clearRefusal: () => setRefusal(null),
    automatic: () => commit(""),
    runTest,
    clearTest: () => setTest(null),
    /* Browse opens in the current path's folder (defaultPath). */
    browse: async () => {
      await wait(120);
      return BROWSED[provider];
    },
  };
}
type Cli = ReturnType<typeof useCli>;

/* ——— shared pieces ——— */

const label = (cli: Cli) => t("settings:cli.label", { provider: cli.name });

function SourceTag({ cli }: { cli: Cli }) {
  const { status } = cli;
  if (!status) return <span className="mk-src">{t("settings:cli.detecting")}</span>;
  if (status.selected_path) return <span className="mk-src custom">{L("custom")}</span>;
  if (!status.source) return null;
  return <span className="mk-src">{L(`src_${status.source}` as CopyKey)}</span>;
}

function VersionTag({ cli }: { cli: Cli }) {
  if (cli.test?.kind !== "version") return null;
  return <span className="mk-ver"><Icon name="check" size={11} />{cli.test.text}</span>;
}

function Receipt({ cli }: { cli: Cli }) {
  if (!cli.receipt) return null;
  return (
    <span className="sync-settings-receipt saved" aria-hidden="true">
      <Icon name="check" size={12} />
      {t("common:saved")}
    </span>
  );
}

function TestButton({ cli, path, disabled }: { cli: Cli; path?: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      className="btn"
      disabled={disabled || cli.testing || (!path && !cli.status?.executable)}
      onClick={() => void cli.runTest(path)}
      title={t("settings:cli.testTitle")}
    >
      {cli.testing ? L("testing") : t("settings:cli.test")}
    </button>
  );
}

/** The one line under a path: the loudest fact wins, said once. */
function Hint({ cli, dirty, extra }: { cli: Cli; dirty?: boolean; extra?: ReactNode }) {
  const custom = Boolean(cli.status?.selected_path);
  const useAuto = custom && (
    <button type="button" className="btn-link" onClick={() => cli.automatic()}>{t("settings:cli.useAutomatic")}</button>
  );
  if (cli.refusal) return <p className="sync-setup-path-hint bad" role="alert">{cli.refusal}</p>;
  if (dirty) return <p className="sync-setup-path-hint">{L("enterToSave")}</p>;
  if (cli.status?.error) return <p className="sync-setup-path-hint bad" role="alert">{cli.status.error}{useAuto}</p>;
  if (cli.test?.kind === "failed") {
    return <p className="sync-setup-path-hint bad" role="alert">{t("settings:cli.testFailed", { message: cli.test.text })}{useAuto}</p>;
  }
  if (!useAuto && !extra) return null;
  return <p className="sync-setup-path-hint">{extra}{useAuto}</p>;
}

/** The project folder field's own markup: a mono value and Browse. */
function PathField({
  cli,
  value,
  placeholder,
  onChange,
  onEnter,
  onBlur,
  autoFocus,
  after,
  children,
}: {
  cli: Cli;
  value: string;
  placeholder: string;
  onChange: (value: string) => void;
  onEnter: () => void;
  onBlur?: () => void;
  autoFocus?: boolean;
  after?: ReactNode;
  children?: ReactNode;
}) {
  const inputId = useId();
  return (
    <div className="sync-project-folder-field">
      <label htmlFor={inputId} className="sync-visually-hidden">{L("pathLabel", { name: cli.name })}</label>
      <div className="sync-profile-select-row">
        <input
          id={inputId}
          value={value}
          placeholder={placeholder}
          autoFocus={autoFocus}
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          aria-invalid={Boolean(cli.refusal || cli.status?.error) || undefined}
          onChange={(event) => onChange(event.target.value)}
          onBlur={onBlur}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onEnter();
            }
          }}
        />
        <button
          type="button"
          className="btn"
          onClick={() => void cli.browse().then((picked) => {
            onChange(picked);
            cli.commit(picked);
          })}
        >
          <Icon name="folder" size={13} /> {L("browse")}
        </button>
        {after}
      </div>
      {children}
    </div>
  );
}

const autoPath = (cli: Cli) => cli.autoFound?.launcher ?? null;
const placeholderOf = (cli: Cli) => {
  const found = autoPath(cli);
  if (found) return L("autoPlaceholder", { path: found });
  return cli.autoFound ? L("nonePlaceholder", { name: cli.name }) : t("settings:cli.detecting");
};

/** Keeps a field's text in step with a save made elsewhere (Browse, Use automatic). */
function useDraft(cli: Cli, from: () => string) {
  const [draft, setDraft] = useState(() => cli.draft0 ?? from());
  const seen = useRef(cli.saved);
  useEffect(() => {
    if (seen.current === cli.saved) return;
    seen.current = cli.saved;
    setDraft(from());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cli.saved]);
  return [draft, setDraft] as const;
}

/* ——— the frames ——— */

function CliCard({ dir, state }: { dir: Exclude<Dir, "before">; state: State }) {
  const codex = useCli("codex", state);
  const claude = useCli("claude", state);
  const Row = ROWS[dir];
  return (
    <main className="sync-main sync-project-links-page sync-settings-page">
      <section className="profile-links-section">
        <section className="sync-settings-group" aria-labelledby={`mk-cli-${dir}`}>
          <h2 className="settings-section-title" id={`mk-cli-${dir}`}>
            <SettingsSectionIcon name="cli" /> {t("settings:sections.cli")}
          </h2>
          <div className="sync-settings-card">
            <Row cli={codex} />
            <Row cli={claude} />
            <div className="sync-settings-row">
              <div className="sync-settings-copy">
                <strong>{t("settings:cli.detection")}</strong>
                <p>{t("settings:cli.detectionHint")}</p>
              </div>
              <div className="sync-settings-control">
                <button type="button" className="btn"><Icon name="refresh" size={14} />{t("settings:cli.refresh")}</button>
              </div>
            </div>
          </div>
        </section>
      </section>
    </main>
  );
}

/** A: the field is the setting. Empty means automatic, and the placeholder
 *  says what automatic found (JetBrains "Auto-detected: …"). */
function RowA({ cli }: { cli: Cli }) {
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? "");
  const dirty = draft.trim() !== (cli.saved ?? "");
  return (
    <div className="sync-settings-row mk-stack mk-a">
      <div className="mk-head"><strong>{label(cli)}</strong><SourceTag cli={cli} /><VersionTag cli={cli} /></div>
      <div className="sync-settings-control"><Receipt cli={cli} /><TestButton cli={cli} /></div>
      <div className="mk-body">
        <PathField
          cli={cli}
          value={draft}
          placeholder={placeholderOf(cli)}
          onChange={(value) => { setDraft(value); cli.clearRefusal(); }}
          onEnter={() => cli.commit(draft)}
        >
          <Hint cli={cli} dirty={dirty && !cli.refusal} />
        </PathField>
      </div>
    </div>
  );
}

/** B: the project setup's agent row — Automatic shows what it found;
 *  Custom path… opens the field. */
function RowB({ cli }: { cli: Cli }) {
  const [mode, setMode] = useState<"auto" | "custom">(cli.saved || cli.draft0 ? "custom" : "auto");
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? "");
  useEffect(() => { if (!cli.saved && !cli.draft0) setMode("auto"); }, [cli.saved, cli.draft0]);
  const dirty = draft.trim() !== (cli.saved ?? "");
  const found = autoPath(cli);
  return (
    <div className="sync-settings-row mk-stack">
      <div className="mk-head"><strong>{label(cli)}</strong><VersionTag cli={cli} /></div>
      <div className="sync-settings-control"><Receipt cli={cli} /><TestButton cli={cli} /></div>
      <div className="mk-body">
        <div className="mk-pick">
          <div className="sync-select-shell">
            <select
              value={mode}
              aria-label={label(cli)}
              onChange={(event) => {
                const next = event.target.value as "auto" | "custom";
                setMode(next);
                if (next === "auto") {
                  setDraft("");
                  cli.automatic();
                }
              }}
            >
              <option value="auto">{L("automatic")}</option>
              <option value="custom">{L("customPath")}</option>
            </select>
          </div>
          {mode === "auto" ? (
            <>
              <span className={`mk-path${found ? "" : " dim"}`}>{found ?? (cli.status ? "—" : t("settings:cli.detecting"))}</span>
              {cli.status?.source && <SourceTag cli={cli} />}
            </>
          ) : (
            <PathField
              cli={cli}
              value={draft}
              autoFocus={!cli.saved && !cli.draft0}
              placeholder={L("pathPlaceholder", { program: cli.program })}
              onChange={(value) => { setDraft(value); cli.clearRefusal(); }}
              onEnter={() => cli.commit(draft)}
            />
          )}
        </div>
        <Hint cli={cli} dirty={mode === "custom" && dirty && !cli.refusal} />
      </div>
    </div>
  );
}

/** C: Xcode's Command Line Tools pop-up — every install found, by path;
 *  Custom path… for one discovery did not meet. */
function RowC({ cli }: { cli: Cli }) {
  const listed = cli.installs.some((install) => install.path === cli.saved);
  const [custom, setCustom] = useState(Boolean(cli.draft0) || (Boolean(cli.saved) && !listed));
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? "");
  const value = custom ? "custom" : cli.saved ?? "auto";
  const dirty = draft.trim() !== (cli.saved ?? "");
  const found = autoPath(cli);
  const where = cli.status?.source && !cli.status.selected_path ? L(`src_${cli.status.source}` as CopyKey) : null;
  return (
    <div className="sync-settings-row mk-stack">
      <div className="mk-head"><strong>{label(cli)}</strong><VersionTag cli={cli} /></div>
      <div className="sync-settings-control"><Receipt cli={cli} /><TestButton cli={cli} /></div>
      <div className="mk-body">
        <div className="sync-select-shell">
          <select
            value={value}
            aria-label={label(cli)}
            className="mk-c-select"
            onChange={(event) => {
              const next = event.target.value;
              if (next === "custom") {
                setCustom(true);
                return;
              }
              setCustom(false);
              cli.commit(next === "auto" ? "" : next);
            }}
          >
            <option value="auto">
              {`${L("automatic")} — ${found ?? (cli.autoFound ? "—" : t("settings:cli.detecting"))}`}
            </option>
            {cli.installs.map((install) => (
              <option key={install.path} value={install.path}>{install.path}</option>
            ))}
            {cli.saved && !listed && !custom && <option value={cli.saved}>{cli.saved}</option>}
            <option value="custom">{L("customPath")}</option>
          </select>
        </div>
        {custom && (
          <PathField
            cli={cli}
            value={draft}
            autoFocus={!cli.draft0}
            placeholder={L("pathPlaceholder", { program: cli.program })}
            onChange={(value) => { setDraft(value); cli.clearRefusal(); }}
            onEnter={() => cli.commit(draft)}
          />
        )}
        <Hint
          cli={cli}
          dirty={custom && dirty && !cli.refusal}
          extra={!custom && where ? <span className="mk-meta"><span className="mk-src">{where}</span></span> : null}
        />
      </div>
    </div>
  );
}

/** D: the row stays a row — the path is its value, Change… opens the field
 *  with Save and Cancel (JetBrains Node runtime, PatternFly inline edit). */
function RowD({ cli }: { cli: Cli }) {
  const [editing, setEditing] = useState(Boolean(cli.draft0));
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? autoPath(cli) ?? "");
  const shown = cli.status?.selected_path ?? cli.status?.launcher ?? null;
  if (editing) {
    return (
      <div className="sync-settings-row mk-stack">
        <div className="mk-head"><strong>{label(cli)}</strong><SourceTag cli={cli} /></div>
        <div className="sync-settings-control"><Receipt cli={cli} /></div>
        <div className="mk-body">
          <PathField
            cli={cli}
            value={draft}
            autoFocus
            placeholder={L("pathPlaceholder", { program: cli.program })}
            onChange={(value) => { setDraft(value); cli.clearRefusal(); }}
            onEnter={() => { if (cli.commit(draft)) setEditing(false); }}
            after={(
              <>
                <button type="button" className="btn" onClick={() => { if (cli.commit(draft)) setEditing(false); }}>{t("common:save")}</button>
                <button type="button" className="btn" onClick={() => { cli.clearRefusal(); setDraft(cli.saved ?? autoPath(cli) ?? ""); setEditing(false); }}>{t("common:cancel")}</button>
              </>
            )}
          >
            {cli.refusal && <p className="sync-setup-path-hint bad" role="alert">{cli.refusal}</p>}
          </PathField>
        </div>
      </div>
    );
  }
  return (
    <div className="sync-settings-row mk-stack">
      <div className="mk-head"><strong>{label(cli)}</strong><SourceTag cli={cli} /><VersionTag cli={cli} /></div>
      <div className="sync-settings-control">
        <Receipt cli={cli} />
        <button type="button" className="btn" onClick={() => setEditing(true)}>{L("change")}</button>
        <TestButton cli={cli} />
      </div>
      <div className="mk-body">
        {shown ? <span className="mk-path">{shown}</span> : !cli.status && <span className="mk-path dim">{t("settings:cli.detecting")}</span>}
        <Hint cli={cli} />
      </div>
    </div>
  );
}

/** E: one editable field holding the path in use, with the installs found
 *  under a caret (HIG combo box, APG editable combobox). */
function RowE({ cli }: { cli: Cli }) {
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? autoPath(cli) ?? "");
  const [open, setOpen] = useState(false);
  const holding = cli.saved ?? autoPath(cli) ?? "";
  const dirty = draft.trim() !== holding;
  const commitTyped = () => {
    if (!dirty) return;
    // The automatic path typed back means automatic, not a pin.
    cli.commit(!cli.saved && draft.trim() === autoPath(cli) ? "" : draft);
  };
  const first = cli.installs[0];
  return (
    <div className="sync-settings-row mk-stack">
      <div className="mk-head"><strong>{label(cli)}</strong><SourceTag cli={cli} /><VersionTag cli={cli} /></div>
      <div className="sync-settings-control"><Receipt cli={cli} /><TestButton cli={cli} /></div>
      <div className="mk-body">
        <div className="mk-combo">
          <PathField
            cli={cli}
            value={draft}
            placeholder={placeholderOf(cli)}
            onChange={(value) => { setDraft(value); cli.clearRefusal(); }}
            onEnter={commitTyped}
            after={(
              <button
                type="button"
                className="btn mk-caret"
                aria-label={L("showInstalls")}
                aria-expanded={open}
                onClick={() => setOpen((value) => !value)}
              >
                <Icon name="chevron-down" size={13} />
              </button>
            )}
          >
            <Hint cli={cli} dirty={dirty && !cli.refusal} />
          </PathField>
          {open && (
            <ul className="mk-combo-list" role="listbox" aria-label={L("showInstalls")}>
              {first && (
                <li>
                  <button type="button" role="option" aria-selected={!cli.saved} onClick={() => { setOpen(false); cli.automatic(); }}>
                    <span className="mk-path">{first.path}</span>
                    <span className="mk-src">{L("autoItem", { where: L(`src_${first.source}` as CopyKey) })}</span>
                  </button>
                </li>
              )}
              {cli.installs.slice(1).map((install) => (
                <li key={install.path}>
                  <button type="button" role="option" aria-selected={cli.saved === install.path} onClick={() => { setOpen(false); cli.commit(install.path); }}>
                    <span className="mk-path">{install.path}</span>
                    <span className="mk-src">{L(`src_${install.source}` as CopyKey)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

/** F: A's field, but nothing saves until the file checks out — the verdict
 *  follows the typing, Test runs the typed path, Save commits it. */
function RowF({ cli }: { cli: Cli }) {
  const [draft, setDraft] = useDraft(cli, () => cli.saved ?? "");
  const [verdict, setVerdict] = useState<ReturnType<typeof checkPath> | null>(null);
  const typed = draft.trim();
  const dirty = typed !== (cli.saved ?? "");
  useEffect(() => {
    if (!dirty || !typed) {
      setVerdict(null);
      return;
    }
    const timer = window.setTimeout(() => setVerdict(checkPath(typed)), 250);
    return () => window.clearTimeout(timer);
  }, [typed, dirty]);
  const ready = dirty && (typed === "" || verdict?.ok === true);
  const save = () => { if (ready) cli.commit(draft); };
  return (
    <div className="sync-settings-row mk-stack">
      <div className="mk-head"><strong>{label(cli)}</strong><SourceTag cli={cli} /><VersionTag cli={cli} /></div>
      <div className="sync-settings-control">
        <Receipt cli={cli} />
        <TestButton cli={cli} path={dirty && verdict?.ok ? typed : undefined} disabled={dirty && !verdict?.ok} />
      </div>
      <div className="mk-body">
        <PathField
          cli={cli}
          value={draft}
          placeholder={placeholderOf(cli)}
          onChange={(value) => { setDraft(value); cli.clearRefusal(); cli.clearTest(); }}
          onEnter={save}
          after={dirty ? <button type="button" className="btn" disabled={!ready} onClick={save}>{t("common:save")}</button> : null}
        >
          {dirty && verdict && (verdict.ok
            ? <p className="sync-setup-path-hint ok"><Icon name="check" size={11} /> {L("executable")}</p>
            : <p className="sync-setup-path-hint bad" role="alert">{verdict.reason}</p>)}
          {!dirty && <Hint cli={cli} />}
        </PathField>
      </div>
    </div>
  );
}

const ROWS: Record<Exclude<Dir, "before">, (props: { cli: Cli }) => ReactNode> = {
  A: RowA, B: RowB, C: RowC, D: RowD, E: RowE, F: RowF,
};

/* ——— Before: the real page on a stand-in backend ——— */

const runtime = window as unknown as { isTauri: boolean };
runtime.isTauri = true;
mockIPC((cmd) => {
  if (cmd === "plugin:app|version") return "0.1.7";
  if (cmd === "plugin:dialog|open") return BROWSED.codex;
  return null;
});

const before = { state: "auto" as State, saved: { codex: null, claude: null } as Record<Provider, string | null> };
const MB = 1024 * 1024;
const settingsOf = (): AppSettings => ({
  codex_cli_path: before.saved.codex,
  claude_cli_path: before.saved.claude,
  push_codex_children: true,
  push_claude_children: true,
  credential_redaction_enabled: false,
  update_check_enabled: true,
  schema: 1,
  conversation_file_limit_bytes: 64 * MB,
  timeline_preview_limit_bytes: 2 * MB,
  comparison_workers: 4,
  cloud_transfer_concurrency: 4,
});
const AGENTS: ProviderDescriptor[] = [
  { key: "codex", display_name: "Codex", display_rank: 10, profile_label: "Codex home", setup_default: true, has_cli: true, maintenance_actions: [] },
  { key: "claude", display_name: "Claude", display_rank: 20, profile_label: "Claude config", setup_default: true, has_cli: true, maintenance_actions: [] },
];
projectSyncApi.listAgentProviders = async () => AGENTS;
projectSyncApi.getAppSettings = async () => settingsOf();
projectSyncApi.updateAppSettings = async (settings) => {
  for (const provider of ["codex", "claude"] as const) {
    const path = settings[`${provider}_cli_path`];
    if (path && !path.startsWith("/")) throw said.notAbsolute(provider);
    before.saved[provider] = path;
  }
  return settingsOf();
};
projectSyncApi.inspectProviderCli = async (provider) => {
  const status = resolveFake(provider as Provider, before.saved[provider as Provider], before.state);
  return status ?? new Promise<ProviderCliStatus>(() => {});
};
projectSyncApi.testProviderCli = async (provider) => {
  await wait(300);
  const status = resolveFake(provider as Provider, before.saved[provider as Provider], before.state);
  const version = status?.executable ? VERSIONS[status.executable] ?? null : null;
  if (!version) throw said.noVersion(provider as Provider);
  return version;
};
projectSyncApi.refreshCliDetection = async () => {};
projectSyncApi.getActivityLogStats = async () => ({
  total_bytes: 3.2 * MB, file_count: 6, oldest_ts: Date.now() - 12 * 86_400_000, newest_ts: Date.now(),
  policy: { schema: 1, retention_days: 30, max_total_bytes: 100 * MB },
});
projectSyncApi.getLocalStateSummary = async () => ({ root_path: `${HOME}/.mallard`, file_count: 41, total_bytes: 18.6 * MB, backups_bytes: 4.1 * MB });

const UPDATES: AppUpdates = {
  readyVersion: null, checking: false, restarting: false, operationRunning: false, lastCheckedAt: null, receiptVersion: null,
  checkNow: () => {}, restart: () => {}, openReceipt: () => {},
};

function BeforeFrame({ state }: { state: State }) {
  before.state = state;
  before.saved = { codex: initialSaved("codex", state), claude: null };
  return (
    <div className="mk-before">
      <SettingsPage appActions={[]} appUpdates={UPDATES} onLogsChanged={() => {}} onDone={() => {}} doneTarget="tidewatch" />
    </div>
  );
}

/* ——— the page ——— */

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    const url = `${window.location.pathname}?${new URLSearchParams({ ...next })}`;
    if (key === "ui") {
      window.location.assign(url);
      return;
    }
    setKnobs(next);
    window.history.replaceState(null, "", url);
  };
  useEffect(() => { document.documentElement.dataset.theme = knobs.theme; }, [knobs.theme]);
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], text: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{text}</button>
  );
  const shown = knobs.only === "all" ? DIRS : [knobs.only];
  return (
    <div className="spec">
      <h1 className="spec-title">Agent CLIs: a path you can type</h1>
      <div className="spec-controls">
        <div className="spec-group"><span>Codex</span>{STATES.map((state) => seg("state", state, STATE_LABEL[state]))}</div>
        <div className="spec-group"><span>Width</span>{seg("width", "760", "760")}{seg("width", "560", "560")}</div>
        <div className="spec-group"><span>UI</span>{seg("ui", "zh-CN", "中文")}{seg("ui", "en", "English")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>
      <p className="spec-seen">
        All: path in mono · source tag · Browse opens in the path&rsquo;s folder (<a href="https://v2.tauri.app/reference/javascript/dialog/#opendialogoptions">dialog defaultPath</a>) · version after Test, as now.
      </p>
      <div className="spec-grid">
        {shown.map((dir) => (
          <div key={dir} data-dir={dir}>
            <h2 className="spec-col-title">{DIR_TITLE[dir]}</h2>
            <p className="spec-seen">
              {DIR_SEEN[dir].seen.map(([name, href], index) => (
                <span key={name}>{index > 0 && " · "}<a href={href}>{name}</a></span>
              ))}
              {DIR_SEEN[dir].against && (
                <span className="spec-against">{DIR_SEEN[dir].seen.length > 0 && " · "}against: {DIR_SEEN[dir].against}</span>
              )}
              {" · "}
              {DIR_SEEN[dir].backend
                ? <span className="spec-backend">backend: {DIR_SEEN[dir].backend}</span>
                : dir !== "before" && <span>no backend change</span>}
            </p>
            <div className="mk-frame" style={{ width: Number(knobs.width) }}>
              {dir === "before"
                ? <BeforeFrame key={knobs.state} state={knobs.state} />
                : <CliCard key={knobs.state} dir={dir} state={knobs.state} />}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

document.documentElement.dataset.theme = readKnobs().theme;
createRoot(document.getElementById("root")!).render(<StrictMode><Specimen /></StrictMode>);

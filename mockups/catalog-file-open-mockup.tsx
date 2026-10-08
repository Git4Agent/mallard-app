import { Fragment, StrictMode, useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Components } from "react-markdown";
import Icon from "../src/components/Icons";
import ManagedCloudChip from "../src/components/managed-cloud/ManagedCloudChip";
import type { ManagedCloudStatus } from "../src/components/managed-cloud/types";
import MarkdownText from "../src/components/markdown/MarkdownText";
import { safeRelativeMarkdownPath } from "../src/components/markdown/markdownUrl";
import StorageCatalogPage, { type CatalogStorageOption } from "../src/components/storage-catalog/StorageCatalogPage";
import { splitSkillFrontmatter } from "../src/components/storage-catalog/skillFiles";
import { storageCatalogApi } from "../src/components/storage-catalog/api";
import { catalogPresentation, catalogScopeLabel } from "../src/components/storage-catalog/model";
import type { CatalogCapabilityDescriptor, CatalogProfile, CatalogScopeRow, CatalogScopeView } from "../src/components/storage-catalog/types";
import ProviderMark from "../src/components/sync-review/ProviderMark";
import SyncPathBar, { type PathBarCrumb } from "../src/components/sync-review/SyncPathBar";
import { formatFullDateTime } from "../src/clock";
import { formatBytes } from "../src/format";
import "../src/App.css";
import "./catalog-file-open-mockup.css";

/**
 * Opening a skill's files on its Catalog item page
 * (docs/PLAN_CATALOG_ITEM_FILES.md). Today the rows are text; each direction
 * opens them its own way, and all of them show one file the same way: the
 * file view at the bottom of this page.
 *
 * Served by the mockups Vite (port 1437), /mockups/catalog-file-open-mockup.html
 * (`?width=580|778|1118&copy=local|stored&open=today|a..g&theme=light|dark`).
 */

const params = new URLSearchParams(window.location.search);
function pick<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  const value = params.get(key);
  return value && (allowed as readonly string[]).includes(value) ? value as T : fallback;
}
const noop = () => {};
const wait = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));
const NOW = Math.floor(Date.now() / 1000);
const EDITED = NOW - 3_600 * 5;

/* ——— the skill ——— */

const SKILL_MD = `---
name: remote-runner
description: Generates and runs maintenance scripts on remote hosts over SSH, with a dry run first.
---

# Remote runner

Write a maintenance script for the host the user names, show it, then run it
over SSH only after the user says yes.

## Steps

1. Read [access notes](references/access.md) for the host's login rules.
2. Run \`python3 scripts/generate.py --host <name> --dry-run\` and show the plan.
3. On approval, \`scripts/remote.py\` copies and runs it.

## Validation

Run \`python3 scripts/test_generate.py\` after code changes. The tests use local
clients and fixtures; confirm live SSH access separately.
`;

const GENERATE_PY = `#!/usr/bin/env python3
"""Generate a maintenance script for one remote host."""

from __future__ import annotations

import argparse
import json
import shlex
import sys
from dataclasses import dataclass
from pathlib import Path

TEMPLATES = Path(__file__).resolve().parent.parent / "references"


@dataclass(frozen=True)
class Host:
    name: str
    user: str
    port: int = 22

    @property
    def target(self) -> str:
        return f"{self.user}@{self.name}"


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", required=True, help="host name from hosts.json")
    parser.add_argument("--dry-run", action="store_true", help="print, do not run")
    parser.add_argument("--steps", nargs="*", default=["disk", "services"])
    return parser.parse_args(argv)


def load_host(name: str) -> Host:
    hosts = json.loads((TEMPLATES / "hosts.json").read_text())
    entry = hosts[name]
    return Host(name=name, user=entry["user"], port=entry.get("port", 22))


def render(host: Host, steps: list[str]) -> str:
    lines = ["#!/bin/sh", "set -eu"]
    for step in steps:
        if step == "disk":
            lines.append("df -h /")
        elif step == "services":
            lines.append("systemctl --failed --no-legend")
        else:
            raise SystemExit(f"unknown step: {step}")
    return "\\n".join(lines) + "\\n"


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    host = load_host(args.host)
    script = render(host, args.steps)
    if args.dry_run:
        sys.stdout.write(script)
        return 0
    print("ssh", shlex.quote(host.target), "-p", host.port)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
`;

const AUTO_PIN_SH = `#!/bin/sh
# Pin the generated script's hash before it is copied to a host.
set -eu

script="\${1:?usage: auto-pin.sh <script>}"
hash=$(shasum -a 256 "$script" | cut -d' ' -f1)
printf '%s  %s\\n' "$hash" "$(basename "$script")" >> pins.txt
echo "pinned $script"
`;

const ACCESS_MD = `# Access notes

- Log in as the service user, never as root.
- Hosts behind the bastion need \`-J bastion\` on every \`ssh\` line.
- A host missing from \`hosts.json\` is out of scope: stop and ask.

| Host group | Login | Port |
| --- | --- | --- |
| build | ci | 22 |
| edge | ops | 2222 |
`;

interface SpecFile {
  path: string;
  size: number;
  exec: boolean;
  /** `null`: not UTF-8 text, shown as a binary. */
  text: string | null;
}

const FILES: SpecFile[] = [
  { path: ".gitignore", size: 24, exec: false, text: "__pycache__/\n*.local.env\n" },
  { path: "SKILL.md", size: 1_412, exec: false, text: SKILL_MD },
  { path: "agents/openai.yaml", size: 277, exec: false, text: "interface:\n  display_name: Remote runner\n  short_description: Generate and run host maintenance scripts\n  default_prompt: Check disk and services on the build hosts.\n" },
  { path: "auto-pin.sh", size: 2_662, exec: true, text: AUTO_PIN_SH },
  { path: "references/access.md", size: 1_228, exec: false, text: ACCESS_MD },
  { path: "references/hosts.json", size: 412, exec: false, text: "{\n  \"build-1\": { \"user\": \"ci\" },\n  \"edge-1\": { \"user\": \"ops\", \"port\": 2222 }\n}\n" },
  { path: "scripts/__pycache__/generate.cpython-312.pyc", size: 28_160, exec: false, text: null },
  { path: "scripts/generate.py", size: 26_624, exec: true, text: GENERATE_PY },
  { path: "scripts/remote.py", size: 15_360, exec: true, text: "#!/usr/bin/env python3\n\"\"\"Copy a pinned script to a host and run it.\"\"\"\n\nimport subprocess\nimport sys\n\n\ndef main() -> int:\n    target, script = sys.argv[1:3]\n    subprocess.run([\"scp\", script, f\"{target}:/tmp/\"], check=True)\n    return subprocess.run([\"ssh\", target, f\"sh /tmp/{script}\"]).returncode\n\n\nif __name__ == \"__main__\":\n    raise SystemExit(main())\n" },
  { path: "scripts/test_generate.py", size: 11_264, exec: true, text: "import unittest\n\nfrom generate import Host, render\n\n\nclass RenderTest(unittest.TestCase):\n    def test_disk_step(self) -> None:\n        self.assertIn(\"df -h /\", render(Host(\"h\", \"u\"), [\"disk\"]))\n\n\nif __name__ == \"__main__\":\n    unittest.main()\n" },
];
const fileAt = (path: string) => FILES.find((file) => file.path === path) ?? null;
const SKILL_NAME = "remote-runner";
const FOLDER = `/Users/you/.codex/skills/${SKILL_NAME}`;
const TOTAL = FILES.reduce((sum, file) => sum + file.size, 0);
const isMarkdown = (path: string) => /\.(md|markdown)$/i.test(path);
const lineCount = (text: string) => text.replace(/\n$/, "").split("\n").length;

/* ——— the machine and the stand-in backend, for Today ——— */

const CODEX: CatalogProfile = {
  profile_id: "profile-codex", provider: "codex", provider_label: "Codex", display_name: "Default Codex",
  configured_home: "/Users/you/.codex", revision: 1, created_at: NOW - 86_400 * 40, updated_at: NOW - 86_400 * 40,
};
const CAPABILITY: CatalogCapabilityDescriptor = {
  provider: "codex", provider_label: "Codex", resource_kind: "agent_skill", format: "agentskills.io.skill", format_version: 1,
  label: "Skills", singular_label: "Skill", presentation: { icon: "skill", kind_label: "Skills", format_label: "Agent Skill" },
  capture: "directory_tree", apply: "automatic", targets: ["user.skills"],
};
const SCOPE: CatalogScopeView = {
  scope: {
    scope_id: "scope-skill", profile_id: CODEX.profile_id, provider: "codex", resource_kind: CAPABILITY.resource_kind,
    format: CAPABILITY.format, format_version: 1, revision: 1, created_at: NOW - 86_400 * 30, updated_at: NOW - 86_400 * 30,
  },
  profile: CODEX, label: CAPABILITY.label, available: true,
  provider_label: "Codex", kind_label: "Skills", format_label: "Agent Skill", capability: CAPABILITY,
};
const STORAGES: CatalogStorageOption[] = [{
  id: "storage-cloud", provider: "managed_cloud", name: "Mallard Cloud", provider_label: "Mallard Cloud", icon: "storage-mallard",
  detail: "maya · mallard-ai.com", detailIsPath: false, account_login: "maya",
}];
const STATUS: ManagedCloudStatus = {
  repository_id: "repo_maya_remote-runner", state: "ready", message: null,
  link: {
    visibility: "private", can_pull: true, repository_id: "repo_maya_remote-runner", website_url: "https://mallard-ai.com/maya/remote-runner",
    owner_login: "maya", name: SKILL_NAME, release: { kind: "private" }, repository_revision: 4,
    can_request_release: true, can_publish_version: true, can_discard_draft: true, role: "owner", can_push: true,
  },
  publication: { draft: "pushed", latest_version: 2, next_version: 3, draft_ahead: 1, public_version: null, review_version: null },
};

type Copy = "local" | "stored";
const COPY: Copy = pick<Copy>("copy", ["local", "stored"], "local");
const local = COPY === "local";

const ROW: CatalogScopeRow = {
  row_id: `row-${SKILL_NAME}`, display_name: SKILL_NAME, resource_id: SKILL_NAME, state: local ? "local_changed" : "local_missing",
  local_present: local, local_path: local ? FOLDER : null, local_modified_at: local ? EDITED : null,
  local_files: local ? FILES.length : null, local_bytes: local ? TOTAL : null,
  remote_present: true, remote_updated_at: NOW - 86_400 * 2, content_sha256: "a3f9".padEnd(64, "0"),
  preview_png: null, description: null,
  warnings: local ? [{ code: "executable_content", message: "scripts/generate.py is executable.", digest: "d".repeat(64) }] : [],
};

const runtime = window as unknown as { isTauri: boolean };
if (!runtime.isTauri) {
  runtime.isTauri = true;
  mockIPC(() => null);
}
storageCatalogApi.listScopeResources = async () => {
  await wait(120);
  return {
    provider_label: "Codex", kind_label: "Skills", format_label: "Agent Skill",
    scope: SCOPE.scope, profile: CODEX, storage_id: "storage-cloud", storage_name: "Mallard Cloud",
    link: { link_id: "link-skill", revision: 3 }, storage_revision: null, rows: [ROW], warnings: [], recovery: [],
    cloud_repositories: { [SKILL_NAME]: STATUS.repository_id! }, cloud_connections: { [SKILL_NAME]: STATUS.repository_id! }, cloud_status: null,
  };
};
storageCatalogApi.getDetail = async () => {
  await wait(150);
  return { kind: "document", path: "SKILL.md", text: SKILL_MD, files: FILES.map(({ path, size, exec }) => ({ path, size, executable: exec })) };
};
storageCatalogApi.getCloudStatus = async () => STATUS;
storageCatalogApi.openProfileApp = async () => {};
storageCatalogApi.openProfileTerminal = async () => {};
storageCatalogApi.listProfiles = async () => [CODEX];
storageCatalogApi.listCapabilities = async () => [CAPABILITY];

function TodayPage() {
  return (
    <div className="ci-app">
      <StorageCatalogPage
        scope={SCOPE} scopes={[SCOPE]} storages={STORAGES} capabilities={[CAPABILITY]}
        appActions={[]} initialStorageId="storage-cloud" initialResourceId={SKILL_NAME}
        onOpenScope={noop} onOpenStorageDetails={noop} onAddStorage={noop} onAddScope={noop} onRemoveScope={noop}
        onProfileRenamed={noop} onScopeGone={noop} notify={noop}
      />
    </div>
  );
}

/* ——— the frame: the shipped bar, the page under it ——— */

type Width = "580" | "778" | "1118";
const PAD: Record<Width, number> = { "580": 28, "778": 38, "1118": 50 };
const frameStyle = (width: Width) => ({ width: Number(width), "--ci-pad": `${PAD[width]}px` } as CSSProperties);

function Bar({ file, onBack, onSwitch }: { file?: string | null; onBack?: () => void; onSwitch?: (path: string) => void }) {
  const glyph = catalogPresentation(CAPABILITY);
  const crumbs: PathBarCrumb[] = [
    { id: "storage", label: "Mallard Cloud", icon: "storage-mallard", tone: "root" },
    { id: "scope", label: catalogScopeLabel(SCOPE), icon: glyph.icon, onActivate: noop },
    { id: "item", label: SKILL_NAME, leading: <Icon name={glyph.icon} size={14} />, onActivate: file ? onBack : undefined },
    ...(file ? [{
      id: "file", label: file.split("/").pop()!, title: file, icon: "file" as const,
      menuHeader: { label: file, detail: formatBytes(fileAt(file)?.size ?? 0), icon: "file" as const },
      menu: [{ label: "Files", items: FILES.filter((each) => each.path !== file).map((each) => ({
        id: each.path, label: each.path, detail: formatBytes(each.size), icon: "file" as const, onSelect: () => onSwitch?.(each.path),
      })) }],
    }] : []),
  ];
  return (
    <SyncPathBar
      crumbs={crumbs}
      actions={(
        <>
          <ManagedCloudChip id="cloud-skill" status={STATUS} storageName="Mallard Cloud" busy={false} refreshing={false}
            onOpenUrl={noop} onDiscardDraft={noop} onRetry={noop} onStorageSettings={noop} />
          <button type="button" className="sync-path-icon-action" aria-label="Refresh comparison">
            <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
          </button>
          <div className="sync-path-action-group" role="group" aria-label="Sync actions">
            <button type="button" className="sync-path-sync" disabled><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span></button>
            <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span><span className="sync-path-count" aria-hidden="true">1</span></button>
          </div>
        </>
      )}
    />
  );
}

function Frame({ width, bar, children, overlay }: { width: Width; bar: ReactNode; children: ReactNode; overlay?: ReactNode }) {
  return (
    <div className="ci-frame" style={frameStyle(width)}>
      <div className="ci-app">
        {bar}
        <main className="sync-main sync-project-links-page sync-git-info-page sync-history-page">
          <section className="profile-links-section" aria-label={SKILL_NAME}>{children}</section>
        </main>
      </div>
      {overlay}
    </div>
  );
}

/* ——— the item page around the body (the shipped classes) ——— */

function ItemActions() {
  return (
    <div className="sync-history-thread-actions catalog-item-actions">
      <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Open in Codex"><ProviderMark provider="codex" displayName="Codex" /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Open in Terminal"><Icon name="terminal" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Fork" disabled={!local}><Icon name="git-fork" size={14} /></button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Reveal in Finder" disabled={!local}><Icon name="folder" size={14} /></button>
    </div>
  );
}

const FACTS: Array<[string, ReactNode]> = local
  ? [["Edited", formatFullDateTime(EDITED)], ["Size", formatBytes(TOTAL)], ["Warnings", "1"]]
  : [["Stored", formatFullDateTime(NOW - 86_400 * 2)]];

function ItemPage({ children, rail = true }: { children: ReactNode; rail?: boolean }) {
  return (
    <div className="catalog-item-page">
      <div className="catalog-item-line">
        <div className="catalog-item-facts-line">
          {FACTS.map(([label, value]) => <span key={label} className="catalog-item-fact" title={label}><Icon name={label === "Size" ? "drive" : label === "Warnings" ? "alert-triangle" : "clock"} size={13} />{value}</span>)}
        </div>
        <ItemActions />
      </div>
      <div className={`catalog-item-hub${rail ? "" : " cf-no-rail"}`}>
        <section className="catalog-item-panel" aria-label={SKILL_NAME}>{children}</section>
        {rail && (
          <aside className="catalog-item-rail" aria-label={`${SKILL_NAME} on this Mac`}>
            <dl className="catalog-item-facts">
              {FACTS.map(([label, value]) => <Fragment key={label}><dt>{label}</dt><dd>{value}</dd></Fragment>)}
            </dl>
            <ItemActions />
          </aside>
        )}
      </div>
    </div>
  );
}

/** SKILL.md as the page renders it, its relative links opening the file. */
function SkillDocument({ onOpen }: { onOpen?: (path: string) => void }) {
  const skill = useMemo(() => splitSkillFrontmatter(SKILL_MD), []);
  const components = useMemo<Components | undefined>(() => onOpen && {
    a: ({ href, children }) => {
      const path = href ? safeRelativeMarkdownPath(href) : null;
      if (!path || !fileAt(path)) return <span>{children}</span>;
      return <a href={`#${path}`} className="cf-md-file-link" onClick={(event) => { event.preventDefault(); onOpen(path); }}>{children}</a>;
    },
  }, [onOpen]);
  return (
    <>
      <p className="catalog-item-lede">{skill.fields.description}</p>
      <div className="catalog-item-document markdown-copy"><MarkdownText text={skill.body} components={components} /></div>
    </>
  );
}

/* ——— the one file view every direction shares ——— */

function Lines({ text }: { text: string }) {
  const lines = text.replace(/\n$/, "").split("\n");
  return (
    <pre className="cf-code" tabIndex={0} aria-label="File contents">
      <code>{lines.map((line, index) => <span key={index} className="cf-line">{line}{"\n"}</span>)}</code>
    </pre>
  );
}

function FileView({ file, named = true, closable, onClose, onOpen }: {
  file: SpecFile;
  /** Off when the bar's crumb already names the file. */
  named?: boolean;
  closable?: boolean;
  onClose?: () => void;
  onOpen?: (path: string) => void;
}) {
  const markdown = isMarkdown(file.path) && file.text != null;
  // SKILL.md opened as a file shows its source: its preview is the item page itself.
  const [rendered, setRendered] = useState(markdown && file.path !== "SKILL.md");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <div className="cf-view">
      <div className="cf-view-head">
        <span className="cf-view-name">
          {named && <><Icon name="file" size={13} /><code>{file.path}</code></>}
          {file.exec && <span className="catalog-item-exec" title="Executable — shown, never run" aria-label="Executable"><Icon name="terminal" size={12} /></span>}
          <span className="cf-view-meta">{file.text == null ? formatBytes(file.size) : `${lineCount(file.text)} lines · ${formatBytes(file.size)}`}</span>
        </span>
        <span className="cf-view-actions">
          {markdown && (
            <span className="sync-segmented" role="group" aria-label="Show">
              <button type="button" aria-pressed={rendered} className={rendered ? "active" : ""} onClick={() => setRendered(true)}>Preview</button>
              <button type="button" aria-pressed={!rendered} className={!rendered ? "active" : ""} onClick={() => setRendered(false)}>Source</button>
            </span>
          )}
          {file.text != null && (
            <button type="button" className="btn catalog-item-copy" onClick={() => setCopied(true)}>
              <Icon name={copied ? "check" : "copy"} size={13} />{copied ? "Copied" : "Copy"}
            </button>
          )}
          <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!local}
            title={local ? `Reveal ${file.path} in Finder` : "Not on this Mac"} aria-label={`Reveal ${file.path} in Finder`}>
            <Icon name="folder" size={14} />
          </button>
          {closable && (
            <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label={`Close ${file.path}`} title="Close" onClick={onClose}>
              <Icon name="x" size={14} />
            </button>
          )}
        </span>
      </div>
      {file.text == null ? (
        <p className="cf-binary"><Icon name="file" size={14} />Binary file · not shown</p>
      ) : markdown && rendered ? (
        file.path === "SKILL.md"
          ? <div className="cf-md"><SkillDocument onOpen={onOpen} /></div>
          : <div className="catalog-item-document markdown-copy cf-md"><MarkdownText text={file.text} /></div>
      ) : (
        <Lines text={file.text} />
      )}
    </div>
  );
}

/* ——— the file list, openable ——— */

function FileRows({ selected, expanded, onOpen, renderOpen, trailing }: {
  selected?: string | null;
  expanded?: ReadonlySet<string>;
  onOpen: (path: string) => void;
  renderOpen?: (file: SpecFile) => ReactNode;
  trailing?: (file: SpecFile) => ReactNode;
}) {
  return (
    <table className="catalog-item-files cf-files" aria-label="Files">
      <tbody>
        {FILES.map((file) => {
          const open = expanded?.has(file.path) ?? false;
          return (
            <Fragment key={file.path}>
              <tr className={selected === file.path || open ? "is-open" : undefined}>
                <td>
                  <button type="button" className="cf-file" aria-expanded={expanded ? open : undefined} aria-current={selected === file.path ? "true" : undefined} onClick={() => onOpen(file.path)}>
                    {expanded ? <Icon name="chevron-right" size={12} /> : <Icon name="file" size={13} />}
                    <code>{file.path}</code>
                  </button>
                  {file.exec && <span className="catalog-item-exec" title="Executable" aria-label="Executable"><Icon name="terminal" size={12} /></span>}
                  {trailing?.(file)}
                </td>
                <td>{formatBytes(file.size)}</td>
              </tr>
              {open && renderOpen && <tr className="cf-expanded"><td colSpan={2}>{renderOpen(file)}</td></tr>}
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
}

/* ——— the directions ——— */

const FIRST = "scripts/generate.py";

/** A · Expand: the row opens the file under itself. */
function PageA({ width }: { width: Width }) {
  const [open, setOpen] = useState<Set<string>>(() => new Set([FIRST]));
  const toggle = (path: string) => setOpen((current) => {
    const next = new Set(current);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });
  return (
    <Frame width={width} bar={<Bar />}>
      <ItemPage>
        <div className="catalog-item-skill">
          <SkillDocument onOpen={(path) => setOpen((current) => new Set(current).add(path))} />
          <FileRows expanded={open} onOpen={toggle} renderOpen={(file) => <FileView file={file} named={false} onOpen={toggle} />} />
        </div>
      </ItemPage>
    </Frame>
  );
}

/** B · Crumb: the file is a page — one more crumb in the bar, its menu the other files. */
function PageB({ width, start = FIRST }: { width: Width; start?: string | null }) {
  const [file, setFile] = useState<string | null>(start);
  const shown = file ? fileAt(file) : null;
  return (
    <Frame width={width} bar={<Bar file={file} onBack={() => setFile(null)} onSwitch={setFile} />}>
      {shown ? (
        <div className="catalog-item-page">
          <section className="catalog-item-panel" aria-label={shown.path}>
            <FileView key={shown.path} file={shown} named={false} onOpen={setFile} />
          </section>
        </div>
      ) : (
        <ItemPage>
          <div className="catalog-item-skill">
            <SkillDocument onOpen={setFile} />
            <FileRows onOpen={setFile} />
          </div>
        </ItemPage>
      )}
    </Frame>
  );
}

/** C · Split: the panel is a list and a view; SKILL.md is the first file. */
function PageC({ width }: { width: Width }) {
  const [file, setFile] = useState(FIRST);
  const shown = fileAt(file)!;
  return (
    <Frame width={width} bar={<Bar />}>
      <ItemPage>
        <div className="cf-split">
          <nav className="cf-split-list" aria-label="Files">
            {FILES.map((each) => (
              <button key={each.path} type="button" className="cf-split-item" aria-current={each.path === file ? "true" : undefined} onClick={() => setFile(each.path)}>
                <Icon name="file" size={13} /><code>{each.path}</code>
              </button>
            ))}
          </nav>
          <FileView key={shown.path} file={shown} onOpen={setFile} />
        </div>
      </ItemPage>
    </Frame>
  );
}

/** D · Tabs: SKILL.md is the first tab, each opened file one more, closable. */
function PageD({ width }: { width: Width }) {
  const [tabs, setTabs] = useState<string[]>([FIRST]);
  const [active, setActive] = useState<string>(FIRST);
  const open = (path: string) => {
    setTabs((current) => (current.includes(path) || path === "SKILL.md" ? current : [...current, path]));
    setActive(path);
  };
  const close = (path: string) => {
    setTabs((current) => current.filter((each) => each !== path));
    setActive((current) => (current === path ? "SKILL.md" : current));
  };
  const shown = active === "SKILL.md" ? null : fileAt(active);
  return (
    <Frame width={width} bar={<Bar />}>
      <ItemPage>
        <div className="cf-tabs-wrap">
          <div className="sync-project-tabs cf-tabs" role="tablist" aria-label="Files open">
            {["SKILL.md", ...tabs].map((path) => (
              <Fragment key={path}>
                <button type="button" role="tab" aria-selected={active === path} className={active === path ? "active" : ""} title={path} onClick={() => setActive(path)}>
                  <Icon name="file" size={13} /><span>{path.split("/").pop()}</span>
                </button>
                {path !== "SKILL.md" && <button type="button" className="cf-tab-close" aria-label={`Close ${path}`} onClick={() => close(path)}><Icon name="x" size={11} /></button>}
              </Fragment>
            ))}
          </div>
          {shown ? <FileView key={shown.path} file={shown} onOpen={open} /> : (
            <div className="catalog-item-skill">
              <SkillDocument onOpen={open} />
              <FileRows selected={active} onOpen={open} />
            </div>
          )}
        </div>
      </ItemPage>
    </Frame>
  );
}

/** E · Quick Look: a sheet over the page; ← → step through the files, Esc closes. */
function PageE({ width }: { width: Width }) {
  const [file, setFile] = useState<string | null>(FIRST);
  const index = file ? FILES.findIndex((each) => each.path === file) : -1;
  const step = (by: number) => setFile(FILES[(index + by + FILES.length) % FILES.length]!.path);
  const shown = file ? fileAt(file) : null;
  return (
    <Frame
      width={width}
      bar={<Bar />}
      overlay={shown && (
        <div className="cf-scrim" onClick={() => setFile(null)}>
          <div className="cf-sheet" role="dialog" aria-modal="true" aria-label={shown.path} onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
              if (event.key === "Escape") setFile(null);
              if (event.key === "ArrowRight") step(1);
              if (event.key === "ArrowLeft") step(-1);
            }}>
            <div className="cf-sheet-head">
              <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Previous file" onClick={() => step(-1)}><Icon name="chevron-left" size={14} /></button>
              <span className="cf-sheet-count">{index + 1} / {FILES.length}</span>
              <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="Next file" onClick={() => step(1)}><Icon name="chevron-right" size={14} /></button>
            </div>
            <FileView key={shown.path} file={shown} closable onClose={() => setFile(null)} onOpen={setFile} />
          </div>
        </div>
      )}
    >
      <ItemPage>
        <div className="catalog-item-skill">
          <SkillDocument onOpen={setFile} />
          <FileRows selected={file} onOpen={setFile} />
        </div>
      </ItemPage>
    </Frame>
  );
}

/** F · Peek: a drawer over the rail; the list stays live beside it. */
function PageF({ width }: { width: Width }) {
  const [file, setFile] = useState<string | null>(FIRST);
  const shown = file ? fileAt(file) : null;
  return (
    <Frame
      width={width}
      bar={<Bar />}
      overlay={shown && (
        <aside className="cf-drawer" aria-label={shown.path}>
          <FileView key={shown.path} file={shown} closable onClose={() => setFile(null)} onOpen={setFile} />
        </aside>
      )}
    >
      <ItemPage>
        <div className="catalog-item-skill">
          <SkillDocument onOpen={setFile} />
          <FileRows selected={file} onOpen={setFile} />
        </div>
      </ItemPage>
    </Frame>
  );
}

/** G · Hand off: no viewer — each row opens in the Mac's text editor or reveals itself. */
function PageG({ width }: { width: Width }) {
  return (
    <Frame width={width} bar={<Bar />}>
      <ItemPage>
        <div className="catalog-item-skill">
          <SkillDocument />
          <FileRows
            onOpen={noop}
            trailing={(file) => (
              <span className="cf-row-actions">
                <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!local || file.text == null}
                  title={!local ? "Not on this Mac" : file.text == null ? "Not text" : "Open in TextEdit"} aria-label={`Open ${file.path} in the text editor`}>
                  <Icon name="external-link" size={13} />
                </button>
                <button type="button" className="btn btn-ghost sync-history-launch-action" disabled={!local} aria-label={`Reveal ${file.path} in Finder`}>
                  <Icon name="folder" size={13} />
                </button>
              </span>
            )}
          />
        </div>
      </ItemPage>
    </Frame>
  );
}

/* ——— the specimen ——— */

type Open = "today" | "a" | "b" | "c" | "d" | "e" | "f" | "g";
interface Direction {
  key: Open;
  title: string;
  pick?: boolean;
  seen: Array<[string, string]>;
  against?: string;
}
const DIRECTIONS: Direction[] = [
  { key: "today", title: "Today", seen: [["Mallard Cloud", "#"]], against: "Rows are text" },
  { key: "b", title: "B · Crumb", pick: true, seen: [["GitHub blob", "https://github.com/anthropics/skills/blob/main/skills/pdf/SKILL.md"], ["Hugging Face", "https://huggingface.co/openai-community/gpt2/blob/main/config.json"], ["HIG path controls", "https://developer.apple.com/design/human-interface-guidelines/path-controls"], ["NN/g breadcrumbs", "https://www.nngroup.com/articles/breadcrumbs/"]], against: "One file at a time" },
  { key: "a", title: "A · Expand", seen: [["GitHub PR files", "https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/reviewing-proposed-changes-in-a-pull-request"], ["APG disclosure", "https://www.w3.org/WAI/ARIA/apg/patterns/disclosure/"]], against: "No file browser does this" },
  { key: "c", title: "C · Split", seen: [["GitHub code view", "https://github.com/anthropics/skills/blob/main/skills/pdf/SKILL.md"], ["docs.rs source", "https://docs.rs/crate/serde/latest/source/"], ["HIG split views", "https://developer.apple.com/design/human-interface-guidelines/split-views"]], against: "Needs width; folds into B" },
  { key: "d", title: "D · Tabs", seen: [["VS Code preview tab", "https://code.visualstudio.com/docs/getstarted/userinterface"], ["HIG tab views", "https://developer.apple.com/design/human-interface-guidelines/tab-views"]], against: "Past six tabs (HIG)" },
  { key: "e", title: "E · Quick Look", seen: [["Finder Quick Look", "https://support.apple.com/guide/mac-help/mh14119/mac"], ["Drive preview", "https://support.google.com/drive/answer/2563044"], ["APG modal", "https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/"]], against: "Hides page; no deep link" },
  { key: "f", title: "F · Peek", seen: [["Notion side peek", "https://www.notion.com/releases/2022-07-20"], ["HIG panels", "https://developer.apple.com/design/human-interface-guidelines/panels"]], against: "Narrow for code lines" },
  { key: "g", title: "G · Hand off", seen: [["Finder", "#"], ["VS Code explorer", "https://code.visualstudio.com/docs/getstarted/userinterface"]], against: "Runs scripts; no stored copy" },
];
const OPENS: readonly Open[] = DIRECTIONS.map((direction) => direction.key);
const PAGES: Record<Exclude<Open, "today">, (props: { width: Width }) => ReactNode> = { a: PageA, b: PageB, c: PageC, d: PageD, e: PageE, f: PageF, g: PageG };

function Seen({ seen, against }: { seen: Array<[string, string]>; against?: string }) {
  return (
    <p className="ci-seen">
      {seen.map(([label, href], index) => <Fragment key={label}>{index > 0 && " · "}{href === "#" ? label : <a href={href} target="_blank" rel="noreferrer">{label}</a>}</Fragment>)}
      {against && <span className="ci-against"><Icon name="alert-triangle" size={11} />{against}</span>}
    </p>
  );
}

const VIEWS: Array<{ title: string; path: string }> = [
  { title: "Code", path: "scripts/generate.py" },
  { title: "Markdown", path: "references/access.md" },
  { title: "Executable", path: "auto-pin.sh" },
  { title: "Binary", path: "scripts/__pycache__/generate.cpython-312.pyc" },
];

type Theme = "light" | "dark";

function Specimen() {
  const [theme, setTheme] = useState<Theme>(pick<Theme>("theme", ["light", "dark"], "light"));
  const [width, setWidth] = useState<Width>(pick<Width>("width", ["580", "778", "1118"], "778"));
  const [open, setOpen] = useState<Open | null>(params.has("open") ? pick<Open>("open", OPENS, "b") : null);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    const query = new URLSearchParams({ width, copy: COPY, theme });
    if (open) query.set("open", open);
    window.history.replaceState(null, "", `?${query.toString()}`);
  }, [width, theme, open]);
  const option = <T extends string>(value: T, current: T, set: (next: T) => void, label: string) => (
    <button key={value} type="button" className="ci-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const page = (key: Open) => {
    if (key === "today") return <div className="ci-frame" style={frameStyle(width)}><TodayPage /></div>;
    const Page = PAGES[key];
    return <Page width={width} />;
  };
  const shown = open ? DIRECTIONS.find((each) => each.key === open)! : null;
  const switchCopy = (next: Copy) => {
    const query = new URLSearchParams(window.location.search);
    query.set("copy", next);
    window.location.search = query.toString();
  };
  return (
    <main className="ci">
      <div className="ci-controls">
        {option("580", width, setWidth, "580")}
        {option("778", width, setWidth, "778")}
        {option("1118", width, setWidth, "1118")}
        <i />
        {option<Copy>("local", COPY, switchCopy, "On this Mac")}
        {option<Copy>("stored", COPY, switchCopy, "Stored only")}
        <i />
        {option("light", theme, setTheme, "Light")}
        {option("dark", theme, setTheme, "Dark")}
      </div>
      {shown ? (
        <section className="ci-open">
          <h2>
            <button type="button" className="btn btn-ghost sync-history-launch-action" aria-label="All" onClick={() => setOpen(null)}><Icon name="chevron-left" size={14} /></button>
            {shown.title}{shown.pick && <em>Pick</em>}
          </h2>
          <Seen seen={shown.seen} against={shown.against} />
          {page(shown.key)}
        </section>
      ) : (
        <>
          <div className="ci-compare">
            {DIRECTIONS.map((direction) => (
              <article key={direction.key} className="ci-cell-page">
                <h3><button type="button" onClick={() => setOpen(direction.key)}>{direction.title}</button>{direction.pick && <em>Pick</em>}</h3>
                <Seen seen={direction.seen} against={direction.against} />
                <div className="ci-zoom" onClick={() => setOpen(direction.key)}>{page(direction.key)}</div>
              </article>
            ))}
          </div>
          <h2>The file view</h2>
          <div className="ci-cases">
            {VIEWS.map((view) => (
              <article key={view.title} className="ci-case">
                <h3>{view.title}</h3>
                <section className="catalog-item-panel"><FileView file={fileAt(view.path)!} /></section>
              </article>
            ))}
          </div>
        </>
      )}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Specimen />
  </StrictMode>,
);

import CatalogRemoveButton, { type CatalogRemovalAction } from "./CatalogRemoveButton";
import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { formatFullDateTime } from "../../clock";
import { formatBytes } from "../../format";
import { errorMessage } from "../../storageError";
import Icon, { type IconName } from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import MarkdownText from "../markdown/MarkdownText";
import Notice from "../notice/Notice";
import { writeClipboardText } from "../sync-review/clipboard";
import ProviderMark from "../sync-review/ProviderMark";
import { storageCatalogApi } from "./api";
import CatalogPetDesk from "./CatalogPetDesk";
import type { CatalogDetailFile, CatalogResourceDetail, CatalogScopeRow } from "./types";

/*
 * A Catalog item's page, made like its page on Mallard Cloud: the item's own
 * body in a panel — a skill's SKILL.md and files, a plugin's store words and
 * install lines, a pet on its desk — with what only this Mac knows in a rail
 * beside it. Each fact is said once: the bar already names the item (its
 * leaf), its kind and provider (the scope crumb), its sync state (Push and
 * Pull) and, on Mallard Cloud, its repository (the Cloud chip), so the page
 * repeats none of them. Under 880px of page the rail becomes one line above
 * the panel.
 */

export type CatalogItemDetailState =
  | { status: "loading" }
  | { status: "ready"; detail: CatalogResourceDetail }
  | { status: "failed"; message: string };

/**
 * The item's page read, once per item, storage and content: a Pull or Push
 * that changes the content asks again, anything else does not.
 */
export function useCatalogItemDetail(scopeId: string, storageId: string | null, row: CatalogScopeRow | null): CatalogItemDetailState | null {
  const resourceId = row?.resource_id ?? null;
  const content = row?.content_sha256 ?? null;
  const localPresent = row?.local_present ?? false;
  const [state, setState] = useState<CatalogItemDetailState | null>(null);
  useEffect(() => {
    if (!resourceId) {
      setState(null);
      return;
    }
    let current = true;
    setState({ status: "loading" });
    storageCatalogApi.getDetail(scopeId, storageId, resourceId)
      .then((detail) => { if (current) setState({ status: "ready", detail }); })
      .catch((reason) => { if (current) setState({ status: "failed", message: errorMessage(reason) }); });
    return () => { current = false; };
  }, [scopeId, storageId, resourceId, content, localPresent]);
  return state;
}

/** The agentskills.io frontmatter, split off SKILL.md as Mallard Cloud's
 *  skill page does: top-level `key: value` lines, indented continuations
 *  folded in (block scalars included), nested maps and lists skipped. Without
 *  a leading `---` and a closing one, the whole file is the body. */
export function splitSkillFrontmatter(text: string): { fields: Record<string, string>; body: string } {
  const lines = text.split(/\r?\n/);
  const close = lines[0] === "---" ? lines.findIndex((line, index) => index > 0 && line.trimEnd() === "---") : -1;
  if (close === -1) return { fields: {}, body: text };
  const fields: Record<string, string> = {};
  for (let index = 1; index < close; index++) {
    const match = /^([A-Za-z0-9_-]+):(.*)$/.exec(lines[index]!);
    if (!match) continue;
    const continuation: string[] = [];
    while (index + 1 < close && /^(\s|$)/.test(lines[index + 1]!)) {
      const line = lines[++index]!.trim();
      if (line) continuation.push(line);
    }
    const value = match[2]!.trim();
    if (/^[|>][+-]?$/.test(value)) fields[match[1]!] = continuation.join(value.startsWith("|") ? "\n" : " ");
    else if (value) fields[match[1]!] = unquote([value, ...continuation].join(" "));
  }
  return { fields, body: lines.slice(close + 1).join("\n") };
}

function unquote(value: string): string {
  const quote = value[0];
  if (value.length < 2 || (quote !== '"' && quote !== "'") || !value.endsWith(quote)) return value;
  const inner = value.slice(1, -1);
  return quote === "'" ? inner.replace(/''/g, "'") : inner.replace(/\\(["\\])/g, "$1");
}

/* ——— the facts and the actions ——— */

/** What only this Mac knows about the item: when it changed and its size,
 *  and the warnings its capture raised. */
function facts(row: CatalogScopeRow): Array<{ icon: IconName; label: string; value: ReactNode; title?: string }> {
  const edited = row.local_present ? row.local_modified_at ?? null : null;
  const stored = edited == null ? row.remote_updated_at ?? null : null;
  const list: Array<{ icon: IconName; label: string; value: ReactNode; title?: string }> = [];
  if (edited != null) list.push({ icon: "clock", label: "Edited", value: <time dateTime={new Date(edited * 1000).toISOString()}>{formatFullDateTime(edited)}</time> });
  if (stored != null) list.push({ icon: "clock", label: "Stored", value: <time dateTime={new Date(stored * 1000).toISOString()}>{formatFullDateTime(stored)}</time> });
  if (row.local_bytes != null) list.push({ icon: "drive", label: "Size", value: formatBytes(row.local_bytes) });
  if (row.warnings.length > 0) {
    list.push({
      icon: "alert-triangle",
      label: "Warnings",
      value: String(row.warnings.length),
      title: row.warnings.map((warning) => warning.message).join("\n"),
    });
  }
  return list;
}

export interface CatalogItemActions {
  removal?: CatalogRemovalAction;
  provider: string;
  providerLabel: string;
  onOpenApp: () => void;
  onOpenTerminal: () => void;
  /** Given for a kind that can be forked; off when nothing is on this Mac. */
  onFork?: () => void;
  forkOpen?: boolean;
  forkTitle?: string;
  onReveal: () => void;
}

/** The card's launch run, right-aligned: the provider app, Terminal, Fork,
 *  Reveal in Finder last. */
function Actions({ row, actions }: { row: CatalogScopeRow; actions: CatalogItemActions }) {
  const canFork = row.local_present && !!row.resource_id;
  return (
    <div className="sync-history-thread-actions catalog-item-actions">
      <CatalogRemoveButton row={row} action={actions.removal} />
      <button type="button" className="btn btn-ghost sync-history-launch-action" title={`Open in ${actions.providerLabel}`} aria-label={`Open in ${actions.providerLabel}`} onClick={actions.onOpenApp}>
        <ProviderMark provider={actions.provider} displayName={actions.providerLabel} />
      </button>
      <button type="button" className="btn btn-ghost sync-history-launch-action" title="Open in Terminal" aria-label="Open in Terminal" onClick={actions.onOpenTerminal}>
        <Icon name="terminal" size={14} />
      </button>
      {actions.onFork && (
        <button
          type="button"
          className="btn btn-ghost sync-history-launch-action"
          disabled={!canFork}
          title={canFork ? actions.forkTitle : "Not on this Mac, so there is nothing to fork"}
          aria-label={canFork ? actions.forkTitle : `Cannot fork ${row.display_name}: not on this Mac`}
          aria-expanded={canFork ? !!actions.forkOpen : undefined}
          onClick={actions.onFork}
        >
          <Icon name="git-fork" size={14} />
        </button>
      )}
      <button
        type="button"
        className="btn btn-ghost sync-history-launch-action"
        disabled={!row.local_path}
        title={row.local_path ? `Reveal ${row.local_path} in Finder` : "Not on this Mac"}
        aria-label={`Reveal ${row.display_name} in Finder`}
        onClick={actions.onReveal}
      >
        <Icon name="folder" size={14} />
      </button>
    </div>
  );
}

/* ——— the bodies ——— */

function FileList({ files }: { files: CatalogDetailFile[] }) {
  return (
    <table className="catalog-item-files" aria-label="Files">
      <tbody>
        {files.map((file) => (
          <tr key={file.path}>
            <td>
              <Icon name="file" size={13} />
              <code>{file.path}</code>
              {file.executable && <span className="catalog-item-exec" title="Executable" aria-label="Executable"><Icon name="terminal" size={12} /></span>}
            </td>
            <td>{formatBytes(file.size)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function SkillBody({ text, path, files, onOpenExternal }: {
  text: string | null;
  path: string;
  files: CatalogDetailFile[];
  onOpenExternal: (url: string) => void;
}) {
  const skill = useMemo(() => (text == null ? null : splitSkillFrontmatter(text)), [text]);
  return (
    <div className="catalog-item-skill">
      {skill?.fields.description && <p className="catalog-item-lede">{skill.fields.description}</p>}
      {skill == null ? (
        <p className="catalog-item-muted">{path} is not UTF-8 text.</p>
      ) : skill.body.trim() ? (
        <div className="catalog-item-document markdown-copy">
          <MarkdownText text={skill.body} onOpenExternal={onOpenExternal} />
        </div>
      ) : (
        <p className="catalog-item-muted">{path} has nothing beyond its frontmatter.</p>
      )}
      <FileList files={files} />
    </div>
  );
}

/** A marketplace source's GitHub page: `owner/repository` shorthand or a
 *  github.com address; anything else has no page to link. */
function githubPage(location: string | null): string | null {
  if (!location) return null;
  if (/^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/.test(location)) return `https://github.com/${location.replace(/\.git$/, "")}`;
  const match = /^https:\/\/github\.com\/([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+?)(\.git)?\/?$/.exec(location);
  return match ? `https://github.com/${match[1]}` : null;
}

/** Mallard Cloud's store hero, for this Mac: the plugin's own words, whether
 *  it is installed here, the lines that install it by hand, and its links. */
function PluginBody({ detail, installedHere, onOpenExternal }: {
  detail: Extract<CatalogResourceDetail, { kind: "install" }>;
  installedHere: boolean;
  onOpenExternal: (url: string) => void;
}) {
  const listing = detail.listing;
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const source = githubPage(detail.location);
  const links: Array<[string, string]> = [];
  for (const [label, href] of [["Website", listing?.website_url], ["Privacy", listing?.privacy_url], ["Terms", listing?.terms_url]] as const) {
    if (href) links.push([label, href]);
  }
  const link = (href: string, children: ReactNode) => (
    <a href={href} rel="noopener noreferrer" onClick={(event) => { event.preventDefault(); onOpenExternal(href); }}>
      {children}<Icon name="external-link" size={10} />
    </a>
  );
  const copy = detail.install.join(" && ");
  return (
    <div className="catalog-item-plugin">
      {(listing?.display_name || listing?.developer || listing?.category) && (
        <p className="catalog-item-by">
          {listing.display_name && <b>{listing.display_name}</b>}
          {listing.developer && <> by {listing.developer}</>}
          {listing.category && <>{listing.display_name || listing.developer ? " · " : ""}{listing.category}</>}
        </p>
      )}
      {listing?.description && <p className="catalog-item-lede">{listing.description}</p>}
      {detail.installed_version ? (
        <p className="catalog-item-here"><Icon name="check-circle" size={14} />Installed · {detail.installed_version}</p>
      ) : !installedHere && (
        <p className="catalog-item-here is-absent"><Icon name="download" size={14} />Pull installs it</p>
      )}
      {detail.install.length > 0 && (
        <div className="catalog-item-install">
          <div className="catalog-item-install-head">
            <span className="catalog-item-install-name">Terminal</span>
            <button
              type="button"
              className="btn catalog-item-copy"
              aria-label={`Copy ${copy}`}
              onClick={() => void writeClipboardText(copy).then(() => setCopied(true), () => undefined)}
            >
              <Icon name={copied ? "check" : "copy"} size={13} />{copied ? "Copied" : "Copy"}
            </button>
          </div>
          <pre className="catalog-item-lines"><code>
            {detail.install.map((line) => (
              <span key={line}><span className="catalog-item-prompt" aria-hidden="true">$ </span>{line}{"\n"}</span>
            ))}
          </code></pre>
        </div>
      )}
      <p className="catalog-item-links">
        {source ? link(source, <><Icon name="link" size={12} />{source.slice("https://github.com/".length)}</>)
          : <code>{detail.location ?? detail.marketplace}</code>}
        {links.map(([label, href]) => <Fragment key={label}>{link(href, label)}</Fragment>)}
      </p>
    </div>
  );
}

function Body({ row, detail, onOpenExternal }: {
  row: CatalogScopeRow;
  detail: CatalogItemDetailState;
  onOpenExternal: (url: string) => void;
}) {
  if (detail.status === "loading") return <LoadingScaffold shape="detail" label={`Loading ${row.display_name}`} />;
  if (detail.status === "failed") {
    return (
      <Notice severity="warning" timing="standing" label={`Why ${row.display_name} could not be shown`} title={`Couldn’t read ${row.display_name}.`}>
        {detail.message}
      </Notice>
    );
  }
  const shown = detail.detail;
  if (shown.kind === "document") return <SkillBody text={shown.text} path={shown.path} files={shown.files} onOpenExternal={onOpenExternal} />;
  if (shown.kind === "install") return <PluginBody detail={shown} installedHere={row.local_present} onOpenExternal={onOpenExternal} />;
  return (
    <div className="catalog-item-pet">
      <CatalogPetDesk atlas={shown.atlas} name={row.display_name} columns={shown.columns} rows={shown.rows} />
      {shown.description && <p className="catalog-item-lede">{shown.description}</p>}
    </div>
  );
}

/* ——— the page ——— */

export default function CatalogItemPage({ row, detail, actions, onOpenExternal, footer }: {
  row: CatalogScopeRow;
  detail: CatalogItemDetailState;
  actions: CatalogItemActions;
  onOpenExternal: (url: string) => void;
  /** The inline fork form, while Fork is open. */
  footer?: ReactNode;
}) {
  const shown = facts(row);
  return (
    <div className="catalog-item-page">
      <div className="catalog-item-line">
        <div className="catalog-item-facts-line">
          {shown.map((fact) => (
            <span key={fact.label} className="catalog-item-fact" title={fact.title ?? fact.label}>
              <Icon name={fact.icon} size={13} />{fact.value}
            </span>
          ))}
        </div>
        <Actions row={row} actions={actions} />
      </div>
      {footer}
      <div className="catalog-item-hub">
        <section className="catalog-item-panel" aria-label={row.display_name}>
          <Body row={row} detail={detail} onOpenExternal={onOpenExternal} />
        </section>
        <aside className="catalog-item-rail" aria-label={`${row.display_name} on this Mac`}>
          {shown.length > 0 && (
            <dl className="catalog-item-facts">
              {shown.map((fact) => (
                <Fragment key={fact.label}>
                  <dt>{fact.label}</dt>
                  <dd title={fact.title}>{fact.value}</dd>
                </Fragment>
              ))}
            </dl>
          )}
          <Actions row={row} actions={actions} />
        </aside>
      </div>
    </div>
  );
}

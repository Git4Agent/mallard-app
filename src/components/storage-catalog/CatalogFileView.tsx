import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { formatBytes, formatNumber } from "../../format";
import { useTranslation } from "../../i18n";
import { errorMessage } from "../../storageError";
import Icon from "../Icons";
import LoadingScaffold from "../LoadingScaffold";
import MarkdownText, { type MarkdownFileLink } from "../markdown/MarkdownText";
import Notice from "../notice/Notice";
import { writeClipboardText } from "../sync-review/clipboard";
import { storageCatalogApi } from "./api";
import { isMarkdownPath, lineCount, splitSkillFrontmatter } from "./skillFiles";
import type { CatalogResourceFile, CatalogScopeRow } from "./types";

/*
 * One of an item's files, opened as its own page (B · Crumb): the bar's last
 * crumb names it, so the view never prints the path. Its head says how big
 * it is and holds the tools; its body is numbered lines, a Markdown preview
 * beside its source, or a word that the file is not text. Nothing here runs
 * or writes the file.
 */

export type CatalogItemFileState =
  | { status: "loading" }
  | { status: "ready"; file: CatalogResourceFile }
  | { status: "failed"; message: string };

/**
 * The open file read, once per item, storage, content and path: a Pull or
 * Push that changes the content reads it again, anything else does not.
 */
export function useCatalogItemFile(
  scopeId: string,
  storageId: string | null,
  row: CatalogScopeRow | null,
  path: string | null,
): CatalogItemFileState | null {
  const resourceId = row?.resource_id ?? null;
  const content = row?.content_sha256 ?? null;
  const localPresent = row?.local_present ?? false;
  const [state, setState] = useState<CatalogItemFileState | null>(null);
  useEffect(() => {
    if (!resourceId || !path) {
      setState(null);
      return;
    }
    let current = true;
    setState({ status: "loading" });
    storageCatalogApi.getFile(scopeId, storageId, resourceId, path)
      .then((file) => { if (current) setState({ status: "ready", file }); })
      .catch((reason) => { if (current) setState({ status: "failed", message: errorMessage(reason) }); });
    return () => { current = false; };
  }, [scopeId, storageId, resourceId, content, localPresent, path]);
  return state;
}

function Lines({ text }: { text: string }) {
  const { t } = useTranslation("library");
  const lines = useMemo(() => text.replace(/\n$/, "").split("\n"), [text]);
  return (
    <pre
      className="catalog-file-code"
      tabIndex={0}
      aria-label={t("file.contents")}
      style={{ "--catalog-file-gutter": `${String(lines.length).length + 1}ch` } as CSSProperties}
    >
      <code>{lines.map((line, index) => <span key={index} className="catalog-file-line">{line}{"\n"}</span>)}</code>
    </pre>
  );
}

function FileBody({ file, fileLink, onOpenExternal, onReveal }: {
  file: CatalogResourceFile;
  fileLink: MarkdownFileLink;
  onOpenExternal: (url: string) => void;
  onReveal: (localPath: string) => void;
}) {
  const { t } = useTranslation(["library", "common"]);
  const markdown = file.text !== null && isMarkdownPath(file.path);
  // SKILL.md opened as a file shows its source: its preview is the item page itself.
  const [preview, setPreview] = useState(markdown && file.path !== "SKILL.md");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);
  const body = useMemo(() => (markdown && file.text !== null ? splitSkillFrontmatter(file.text).body : null), [markdown, file.text]);
  const lines = file.text === null ? null : lineCount(file.text);
  return (
    <div className="catalog-file-view">
      <div className="catalog-file-head">
        <span className="catalog-file-facts">
          {file.executable && <span className="catalog-item-exec" title={t("file.executableTitle")} aria-label={t("item.executable")}><Icon name="terminal" size={12} /></span>}
          <span>{lines === null ? formatBytes(file.size) : t("file.lines", { count: lines, lines: formatNumber(lines), size: formatBytes(file.size) })}</span>
        </span>
        <span className="catalog-file-tools">
          {markdown && (
            <span className="sync-segmented" role="group" aria-label={t("file.show")}>
              <button type="button" aria-pressed={preview} className={preview ? "active" : undefined} onClick={() => setPreview(true)}>{t("file.preview")}</button>
              <button type="button" aria-pressed={!preview} className={preview ? undefined : "active"} onClick={() => setPreview(false)}>{t("file.source")}</button>
            </span>
          )}
          {file.text !== null && (
            <button
              type="button"
              className="btn catalog-item-copy"
              aria-label={t("file.copyContents", { path: file.path })}
              onClick={() => void writeClipboardText(file.text!).then(() => setCopied(true), () => undefined)}
            >
              <Icon name={copied ? "check" : "copy"} size={13} />{copied ? t("common:copied") : t("common:copy")}
            </button>
          )}
          <button
            type="button"
            className="btn btn-ghost sync-history-launch-action"
            disabled={!file.local_path}
            title={file.local_path ? t("card.reveal", { target: file.local_path }) : t("card.notOnMac")}
            aria-label={t("card.reveal", { target: file.path })}
            onClick={() => file.local_path && onReveal(file.local_path)}
          >
            <Icon name="folder" size={14} />
          </button>
        </span>
      </div>
      {file.text === null ? (
        <p className="catalog-file-none"><Icon name="file" size={14} />{t("file.binary")}</p>
      ) : preview && body !== null ? (
        body.trim()
          ? <div className="catalog-item-document markdown-copy"><MarkdownText text={body} fileLink={fileLink} onOpenExternal={onOpenExternal} /></div>
          : <p className="catalog-item-muted">{t("item.onlyFrontmatter", { path: file.path })}</p>
      ) : (
        <Lines text={file.text} />
      )}
    </div>
  );
}

export default function CatalogFileView({ path, state, fileLink, onOpenExternal, onReveal }: {
  path: string;
  state: CatalogItemFileState;
  /** Relative links in a previewed Markdown file, resolved against this file. */
  fileLink: MarkdownFileLink;
  onOpenExternal: (url: string) => void;
  onReveal: (localPath: string) => void;
}) {
  const { t } = useTranslation("library");
  if (state.status === "loading") return <LoadingScaffold shape="detail" label={t("page.loading", { label: path })} />;
  if (state.status === "failed") {
    return (
      <Notice severity="warning" timing="standing" label={t("item.whyUnreadable", { name: path })} title={t("item.unreadable", { name: path })}>
        {state.message}
      </Notice>
    );
  }
  return <FileBody key={state.file.path} file={state.file} fileLink={fileLink} onOpenExternal={onOpenExternal} onReveal={onReveal} />;
}

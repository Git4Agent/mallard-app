import type { ManagedCloudStatus } from "./types";
import { useEffect, useState } from "react";
import Icon, { type IconName } from "../Icons";
import { writeClipboardText } from "../sync-review/clipboard";
import { PathBarPopover } from "../sync-review/SyncPathBar";

import {
  canDiscardDraft, canPublishVersion, cloudContributor, cloudFacts, cloudHeadline, cloudReleaseUrl, cloudStatusNotice, cloudVerbs, cloudVersionUrl,
  cloudVersionsUrl, discardDraftHold, discardDraftQuestion, publishVersionHold,
} from "./managedCloudLink";

/**
 * The Mallard Cloud chip in the path bar (docs/PLAN_CLOUD_STATUS_CHIP.md): one
 * mark and word for whatever the service answered, and a panel under it with
 * the repository's facts and verbs. Every link row opens its page, and the
 * button at its end copies the address. The words are the pure functions in
 * managedCloudLink.ts; the workspace hands in what each verb does.
 */

export interface ManagedCloudChipActions {
  onOpenUrl: (url: string) => void;
  /** A project's repository publishes versions here and lists them on the
   *  website. A Catalog item's page has neither, so the Library leaves this
   *  out, and its panel offers no Publish and no Versions. */
  onPublishVersion?: () => void;
  onDiscardDraft: () => void;
  onRetry: () => void;
  onStorageSettings: () => void;
}

interface PanelProps extends ManagedCloudChipActions {
  status: ManagedCloudStatus;
  storageName: string;
  /** A Push, Pull or Cloud request is running: Publish and the discard wait. */
  busy: boolean;
  refreshing: boolean;
  close: (returnFocus: boolean) => void;
}

interface CloudRow {
  id: string;
  label: string;
  icon: IconName;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  onSelect: () => void;
  copy?: { name: string; url: string };
}

/** How long a row's copy button says Copied, as File info's rows do. */
const COPIED_FOR_MS = 2_000;

export function ManagedCloudPanel({
  status, storageName, busy, refreshing, close, onOpenUrl, onPublishVersion, onDiscardDraft, onRetry, onStorageSettings,
}: PanelProps) {
  const [copied, setCopied] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(null), COPIED_FOR_MS);
    return () => window.clearTimeout(timeout);
  }, [copied]);
  const copy = async (name: string, url: string) => {
    try {
      await writeClipboardText(url);
      setCopyFailed(false);
      setCopied(name);
    } catch {
      setCopied(null);
      setCopyFailed(true);
    }
  };

  const head = cloudHeadline(status);
  const link = status.state === "ready" ? status.link : null;
  const rows: CloudRow[] = [];
  if (link) {
    const verbs = cloudVerbs(status);
    /* A verb that leaves you on this page hands focus back to the chip, as a
       crumb menu does on activation; only Storage settings navigates away. */
    const open = (url: string) => () => {
      close(true);
      onOpenUrl(url);
    };
    if (onPublishVersion && verbs.publishVersion !== null) {
      rows.push({
        id: "publish-version", label: `Publish version ${verbs.publishVersion}…`, icon: "git-commit",
        disabled: busy || !canPublishVersion(status), title: publishVersionHold(status) ?? undefined,
        onSelect: () => {
          close(true);
          onPublishVersion();
        },
      });
    }
    const release = { name: "release link", url: cloudReleaseUrl(link) };
    if (verbs.requestRelease) {
      rows.push({ id: "request-release", label: "Request public release", icon: "check-circle", copy: release, onSelect: open(release.url) });
    } else if (verbs.viewRelease) {
      rows.push({ id: "view-release", label: "View release request", icon: "external-link", copy: release, onSelect: open(release.url) });
    }
    if (verbs.viewDraft) {
      const url = cloudVersionUrl(link, "draft");
      rows.push({ id: "view-draft", label: "View draft", icon: "external-link", copy: { name: "draft link", url }, onSelect: open(url) });
    }
    if (onPublishVersion && verbs.versions) {
      const url = cloudVersionsUrl(link);
      rows.push({ id: "versions", label: "Versions", icon: "external-link", copy: { name: "versions link", url }, onSelect: open(url) });
    }
    if (verbs.viewPublic !== null) {
      const url = cloudVersionUrl(link, verbs.viewPublic);
      rows.push({ id: "view-public", label: "View public version", icon: "external-link", copy: { name: "public link", url }, onSelect: open(url) });
    }
    if (verbs.discardDraft) {
      // In view for the Owner and Admins in every state; off, with its reason, while there is nothing to remove.
      const { kept } = discardDraftQuestion(link, status.publication);
      rows.push({
        id: "discard-draft", label: "Discard draft…", icon: "x", danger: true, disabled: busy || !canDiscardDraft(status),
        title: discardDraftHold(status) ?? (kept ? `Discard draft… · back to ${kept}` : "Discard draft… · every push"),
        onSelect: () => {
          close(true);
          onDiscardDraft();
        },
      });
    }
  }
  const note = link ? null : cloudStatusNotice(status);
  const plain = rows.filter((row) => !row.danger);
  const danger = rows.filter((row) => row.danger);

  const renderRow = (row: CloudRow) => {
    const item = (
      <button
        key={row.copy ? undefined : row.id}
        type="button"
        className={`sync-path-menu-item${row.danger ? " danger" : ""}`}
        aria-disabled={row.disabled || undefined}
        title={row.title}
        onClick={() => {
          if (!row.disabled) row.onSelect();
        }}
      >
        <Icon name={row.icon} size={14} />
        <span className="sync-path-menu-copy"><span>{row.label}</span></span>
      </button>
    );
    if (!row.copy) return item;
    const target = row.copy;
    const done = copied === target.name;
    return (
      <div key={row.id} className="sync-cloud-panel-row">
        {item}
        <button
          type="button"
          className={`sync-cloud-panel-copy${done ? " copied" : ""}`}
          aria-label={`Copy ${target.name}`}
          title={`Copy ${target.name}\n${target.url}`}
          onClick={() => void copy(target.name, target.url)}
        >
          {done && <small>Copied</small>}
          <Icon name={done ? "check" : "copy"} size={13} />
        </button>
      </div>
    );
  };

  return (
    <div className="sync-cloud-panel">
      <div className={`sync-cloud-panel-head ${head.tone}`}>
        <Icon name={head.mark} size={24} />
        <span className="sync-path-menu-copy">
          <span>{head.word}</span>
          <small>{link ? `${link.owner_login}/${link.name}` : storageName}</small>
        </span>
      </div>
      <p className="sync-cloud-panel-line">{head.line}</p>
      {link && (
        <dl className="sync-cloud-panel-facts">
          {cloudFacts(link, status.publication).map((fact) => (
            <div key={fact.label}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>
          ))}
        </dl>
      )}
      {link && cloudContributor(link) && (
        <p className="sync-cloud-panel-line quiet">Not a collaborator: anyone signed in can push here, and the Owner decides what's released.</p>
      )}
      {note?.action && (
        <button
          type="button"
          className="btn btn-ghost sync-cloud-panel-action"
          disabled={note.action === "retry" && refreshing}
          onClick={() => {
            close(note.action !== "storage_settings");
            if (note.action === "storage_settings") onStorageSettings();
            else onRetry();
          }}
        >
          {note.action === "storage_settings" ? "Storage settings" : "Retry"}
        </button>
      )}
      {rows.length > 0 && (
        /* A group, not a menu: a row with a copy button is two buttons, which
           a menuitem cannot hold. Tab walks it. */
        <div className="sync-path-menu-list sync-cloud-panel-verbs" role="group" aria-label="Mallard Cloud actions">
          <div className="sync-path-menu-separator" role="separator" />
          {plain.map(renderRow)}
          {danger.length > 0 && plain.length > 0 && <div className="sync-path-menu-separator" role="separator" />}
          {danger.map(renderRow)}
        </div>
      )}
      <span className="sync-visually-hidden" role="status" aria-live="polite">
        {copyFailed ? "Couldn't copy the link." : copied ? `Copied the ${copied}.` : ""}
      </span>
      {copyFailed && (
        <p className="sync-cloud-panel-line error" aria-hidden="true">Couldn't copy. The address is in the button's tooltip.</p>
      )}
    </div>
  );
}

export default function ManagedCloudChip({
  status, storageName, busy, refreshing, id = "managed-cloud", ...actions
}: ManagedCloudChipActions & {
  id?: string;
  /** Null while the first answer is on its way: the chip checks, inert. */
  status: ManagedCloudStatus | null;
  storageName: string;
  busy: boolean;
  refreshing: boolean;
}) {
  const head = cloudHeadline(status);
  const name = `Mallard Cloud: ${head.word}. ${head.line}`;
  return (
    <PathBarPopover
      id={id}
      label="Mallard Cloud"
      className={`sync-path-attention sync-path-cloud-status ${status ? head.tone : "checking"}`}
      ariaLabel={name}
      tooltip={name}
      disabled={!status}
      face={(
        <>
          <Icon name={head.mark} size={16} />
          <b>{head.word}</b>
          {status && <Icon name="chevron-down" size={11} className="sync-path-chevron" />}
        </>
      )}
    >
      {(close) => status && (
        <ManagedCloudPanel status={status} storageName={storageName} busy={busy} refreshing={refreshing} close={close} {...actions} />
      )}
    </PathBarPopover>
  );
}

import { useEffect, useState } from "react";
import Icon from "../Icons";
import { writeClipboardText } from "../sync-review/clipboard";

/** How long Copy says Copied, as the Cloud panel's rows do. */
const COPIED_FOR_MS = 2_000;

/**
 * A settled Mallard Cloud storage's address, leading its settings card: the
 * service in the display face with Copy and Open website beside it, and the
 * account the storage signs in as under it. The host opens too, for the
 * pointer; the keyboard has Open website. The caller hands in how a page
 * opens, so this stays free of the webview.
 */
export default function ManagedCloudAddress({ account, onOpenUrl }: {
  /** The saved service origin, its host, and the account's login. */
  account: { login: string; origin: string; host: string };
  onOpenUrl: (url: string) => void;
}) {
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(false), COPIED_FOR_MS);
    return () => window.clearTimeout(timeout);
  }, [copied]);
  const copy = async () => {
    try {
      await writeClipboardText(account.origin);
      setCopyFailed(false);
      setCopied(true);
    } catch {
      setCopied(false);
      setCopyFailed(true);
    }
  };
  const open = () => onOpenUrl(account.origin);
  // The scheme recedes; the host is what the browser's address bar shows.
  const scheme = account.origin.slice(0, account.origin.length - account.host.length);
  return (
    <section className="managed-cloud-address" aria-label="Mallard Cloud website">
      <div className="managed-cloud-address-bar">
        <span className="managed-cloud-address-mark"><Icon name="link" size={16} /></span>
        <button
          type="button"
          className="managed-cloud-address-url"
          tabIndex={-1}
          title={`Open ${account.origin} in your browser`}
          onClick={open}
        >
          <span className="managed-cloud-address-scheme">{scheme}</span>
          <span className="managed-cloud-address-host">{account.host}</span>
        </button>
        <span className="managed-cloud-address-verbs">
          <button
            type="button"
            className={`btn${copied ? " storage-setup-copied-flash" : ""}`}
            title={`Copy ${account.origin}`}
            onClick={() => void copy()}
          >
            <Icon name={copied ? "check" : "copy"} size={13} /> {copied ? "Copied" : "Copy"}
          </button>
          <button type="button" className="btn" title={`Open ${account.origin} in your browser`} onClick={open}>
            <Icon name="external-link" size={13} /> Open website
          </button>
        </span>
      </div>
      <p className="managed-cloud-address-account">
        <Icon name="user" size={12} /> Signed in as <b>{account.login}</b>
      </p>
      <span className="sync-visually-hidden" role="status" aria-live="polite">
        {copyFailed ? "Couldn't copy the address." : copied ? "Copied the address." : ""}
      </span>
      {copyFailed && (
        <p className="managed-cloud-address-error" aria-hidden="true">Couldn't copy the address.</p>
      )}
    </section>
  );
}

import { StrictMode, useRef, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import Icon from "./components/Icons";
import {
  isCloudflareR2AccountId,
  isCloudflareR2BucketName,
  parseR2Endpoint,
  parseR2S3ApiUrl,
  r2EndpointForAccount,
  type R2Jurisdiction,
} from "./components/project-sync/StorageSettings";
import "./App.css";
import "./RetroTheme.css";

/**
 * The R2 storage setup — the agent action DECIDED, the field model open.
 *
 * The agent action is settled: no banner; an accent-tinted button in the
 * guide row — `--accent` on `--accent-soft`, the pair the storage icon
 * already wears — so the route is unmissable while Create keeps the card's
 * only primary. The shipped mechanic is intact: the click copies a prompt
 * for a terminal coding agent; the agent drives Mallard's packaged CLI while
 * the user types the secret in the CLI's hidden prompt (it never passes
 * through the agent); and the open page listens, adopting the committed
 * storage and filling itself in. After the click the button's slot carries
 * that lifecycle — listening with a loader, then the connected note — and in
 * this specimen the CLI finish is faked seconds later so all three states
 * are real on screen.
 *
 * Below that, the open field model question (A–D); the decided row composes
 * with any of them.
 *
 * Everything wears the settled text diet: no `?` help buttons, no
 * `Example:` prefixes, the shown-once warning captioning the keys it is
 * about. Address parsing is imported from the shipped StorageSettings, so
 * every verdict here is the app's own judgment. Not part of the app bundle:
 * the build has one entry, and this file is reachable only from the dev
 * server.
 */

/* ── the verdict on a pasted address, judged by the shipped parsers ──────── */

type Verdict =
  | { tone: "mute" }
  | { tone: "ok"; accountId: string; bucket: string; jurisdiction: R2Jurisdiction }
  | { tone: "warn"; accountId: string; jurisdiction: R2Jurisdiction }
  | { tone: "bad" };

function judgeS3ApiUrl(raw: string): Verdict {
  const value = raw.trim();
  if (!value) return { tone: "mute" };
  const parsed = parseR2S3ApiUrl(value);
  if (parsed) return { tone: "ok", ...parsed };
  const endpoint = parseR2Endpoint(value);
  if (endpoint) return { tone: "warn", ...endpoint };
  return { tone: "bad" };
}

function shortAccount(accountId: string): string {
  return `${accountId.slice(0, 8)}…${accountId.slice(-8)}`;
}

function jurisdictionTail(jurisdiction: R2Jurisdiction): ReactNode {
  if (jurisdiction === "default") return null;
  return <> · {jurisdiction.toUpperCase()} jurisdiction</>;
}

function OkLine({ verdict }: { verdict: Extract<Verdict, { tone: "ok" }> }) {
  return (
    <p className="sync-r2-verdict ok" aria-live="polite">
      Bucket <code>{verdict.bucket}</code> · account <code>{shortAccount(verdict.accountId)}</code>
      {jurisdictionTail(verdict.jurisdiction)}
    </p>
  );
}

function BadLine() {
  return (
    <p className="sync-r2-verdict bad" aria-live="polite">
      Not an R2 S3 API URL — expected <code>https://&lt;account-id&gt;.r2.cloudflarestorage.com/&lt;bucket&gt;</code>.
    </p>
  );
}

/* ── shared card chrome ──────────────────────────────────────────────────── */

function CardHeader({ name, onName }: { name: string; onName: (next: string) => void }) {
  return (
    <header>
      <span className="sync-storage-icon"><Icon name="cloud" size={18} /></span>
      <input value={name} onChange={(event) => onName(event.target.value)} aria-label="Storage name" />
      <div className="sync-segmented" role="radiogroup" aria-label="Storage type">
        <button type="button" role="radio" aria-checked="true" className="active">Cloudflare R2</button>
        <button type="button" role="radio" aria-checked="false">Local folder</button>
      </div>
    </header>
  );
}

function GuideRow({ extra }: { extra?: ReactNode }) {
  return (
    <div className="sync-r2-guide" role="note">
      <Icon name="info" size={13} />
      <span className="sync-r2-guide-copy">
        Needs a bucket and an <strong>Object Read &amp; Write</strong> token scoped to it.
      </span>
      <button type="button" className="btn-link">Setup guide</button>
      {extra}
    </div>
  );
}

function KeysPair({ accessKeyId, onAccessKeyId, secret, onSecret }: {
  accessKeyId: string;
  onAccessKeyId: (next: string) => void;
  secret: string;
  onSecret: (next: string) => void;
}) {
  const [visible, setVisible] = useState(false);
  return (
    <>
      <label>
        <span className="sync-field-name">Access Key ID</span>
        <input
          value={accessKeyId}
          onChange={(event) => onAccessKeyId(event.target.value)}
          placeholder="4a2b…91ef"
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
        />
      </label>
      <label>
        <span className="sync-field-name">Secret Access Key</span>
        <span className="sync-secret-input">
          <input
            type={visible ? "text" : "password"}
            value={secret}
            onChange={(event) => onSecret(event.target.value)}
            placeholder="a1B2…x9Y0"
            autoComplete="new-password"
          />
          <button
            type="button"
            className="sync-secret-reveal"
            onClick={() => setVisible((current) => !current)}
            aria-pressed={visible}
          >
            {visible ? "Hide" : "Show"}
          </button>
        </span>
      </label>
      <p className="sync-r2-keys-hint">Both are shown once, when the token is created.</p>
    </>
  );
}

function CommitBar({ set, of, ready }: { set: number; of: number; ready: boolean }) {
  return (
    <footer className="sync-storage-setup-footer">
      <span className="sync-storage-editor-progress" aria-live="polite">{set} of {of} set</span>
      <div>
        <button type="button" className="btn btn-ghost">Cancel</button>
        <button type="button" className="btn btn-primary" disabled={!ready}>Create storage</button>
      </div>
    </footer>
  );
}

/* ── model A's draft, liftable so the agent lifecycle can fill it ────────── */

interface Seed {
  name: string;
  url?: string;
  bucket?: string;
  accountId?: string;
  jurisdiction?: R2Jurisdiction;
  accessKeyId?: string;
  secret?: string;
}

function useR2Draft(seed: Seed) {
  const [name, setName] = useState(seed.name);
  const [url, setUrl] = useState(seed.url ?? "");
  const [accessKeyId, setAccessKeyId] = useState(seed.accessKeyId ?? "");
  const [secret, setSecret] = useState(seed.secret ?? "");
  const verdict = judgeS3ApiUrl(url);
  const set = [verdict.tone === "ok", !!accessKeyId.trim(), !!secret.trim()].filter(Boolean).length;
  const fill = (next: { url: string; accessKeyId: string; secret: string }) => {
    setUrl(next.url);
    setAccessKeyId(next.accessKeyId);
    setSecret(next.secret);
  };
  return {
    name, setName, url, setUrl, accessKeyId, setAccessKeyId, secret, setSecret,
    verdict, set, ready: set === 3 && !!name.trim(), fill,
  };
}

type R2DraftState = ReturnType<typeof useR2Draft>;

function UrlFields({ draft }: { draft: R2DraftState }) {
  return (
    <div className="sync-storage-fields sync-r2-fields">
      <label className="sync-r2-url-field">
        <span className="sync-field-name">S3 API URL</span>
        <input
          type="url"
          value={draft.url}
          onChange={(event) => draft.setUrl(event.target.value)}
          placeholder="https://<account-id>.r2.cloudflarestorage.com/<bucket>"
          spellCheck={false}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          aria-invalid={draft.verdict.tone === "bad" || draft.verdict.tone === "warn" || undefined}
        />
        {draft.verdict.tone === "mute" && (
          <p className="sync-r2-verdict mute">Copy it from the bucket&rsquo;s Settings page.</p>
        )}
        {draft.verdict.tone === "ok" && <OkLine verdict={draft.verdict} />}
        {draft.verdict.tone === "warn" && (
          <p className="sync-r2-verdict warn" aria-live="polite">
            That&rsquo;s the account endpoint — add <code>/your-bucket</code> to make it a bucket address.
          </p>
        )}
        {draft.verdict.tone === "bad" && <BadLine />}
      </label>
      <KeysPair
        accessKeyId={draft.accessKeyId}
        onAccessKeyId={draft.setAccessKeyId}
        secret={draft.secret}
        onSecret={draft.setSecret}
      />
    </div>
  );
}

function UrlCard({ draft, guideExtra }: {
  draft: R2DraftState;
  /** The agent action's slot at the guide row's tail. */
  guideExtra?: ReactNode;
}) {
  return (
    <div className="sync-storage-details-panel">
      <article className="sync-storage-card">
        <CardHeader name={draft.name} onName={draft.setName} />
        <GuideRow extra={guideExtra} />
        <UrlFields draft={draft} />
      </article>
      <CommitBar set={draft.set} of={3} ready={draft.ready} />
    </div>
  );
}

/* ── the simulated agent run, shared by every volume ─────────────────────── */

const ACCOUNT = "023e105f4ecef8ad9ca31a8372d0c353";
const EU_URL = `https://${ACCOUNT}.eu.r2.cloudflarestorage.com/codex-sync-backups`;
const BARE_ENDPOINT = `https://${ACCOUNT}.r2.cloudflarestorage.com`;
const KEY_ID = "4a2b8c1d9e0f42a1b7c3d5e6f708192a";
const AGENT_FILL = { url: EU_URL, accessKeyId: KEY_ID, secret: "committed-by-cli" };

type AgentPhase = "offer" | "waiting" | "connected";

function useSimulatedAgent(draft: R2DraftState) {
  const [phase, setPhase] = useState<AgentPhase>("offer");
  const timer = useRef<number | null>(null);
  const copy = () => {
    setPhase("waiting");
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      draft.fill(AGENT_FILL);
      setPhase("connected");
    }, 2800);
  };
  return { phase, copy };
}

/** The row tail once the offer is taken: identical in every volume, so the
 *  variants differ only in how the offer itself is drawn. */
function RowTailStates({ phase }: { phase: Exclude<AgentPhase, "offer"> }) {
  return phase === "waiting" ? (
    <span className="sync-r2-guide-wait" role="status">
      <span className="status-loader" /> Paste into your agent — listening…
    </span>
  ) : (
    <span className="sync-r2-guide-connected" role="status">
      <Icon name="check-circle" size={13} /> Connected — filled in automatically
    </span>
  );
}

/* ── the decided agent action: the accent button in the guide row ────────── */

function VolumeAccent() {
  const draft = useR2Draft({ name: "R2 storage 2" });
  const agent = useSimulatedAgent(draft);
  return (
    <div className="mockup-column">
      <UrlCard
        draft={draft}
        guideExtra={agent.phase === "offer" ? (
          <button type="button" className="btn sync-r2-guide-btn sync-r2-agent-cta" onClick={agent.copy}>
            <Icon name="terminal" size={13} /> Copy agent instruction
          </button>
        ) : <RowTailStates phase={agent.phase} />}
      />
    </div>
  );
}

/* ── B · address, then the missing part ──────────────────────────────────── */

function ModelReveal({ seed }: { seed: Seed }) {
  const [name, setName] = useState(seed.name);
  const [url, setUrl] = useState(seed.url ?? "");
  const [bucket, setBucket] = useState(seed.bucket ?? "");
  const [accessKeyId, setAccessKeyId] = useState(seed.accessKeyId ?? "");
  const [secret, setSecret] = useState(seed.secret ?? "");
  const verdict = judgeS3ApiUrl(url);
  const revealed = verdict.tone === "warn";
  const bucketOk = isCloudflareR2BucketName(bucket);
  const addressSet = verdict.tone === "ok" || (revealed && bucketOk);
  const set = [addressSet, !!accessKeyId.trim(), !!secret.trim()].filter(Boolean).length;
  return (
    <div className="sync-storage-details-panel">
      <article className="sync-storage-card">
        <CardHeader name={name} onName={setName} />
        <GuideRow />
        <div className="sync-storage-fields sync-r2-fields">
          <label className="sync-r2-url-field">
            <span className="sync-field-name">S3 API URL</span>
            <input
              type="url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://<account-id>.r2.cloudflarestorage.com/<bucket>"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
              aria-invalid={verdict.tone === "bad" || undefined}
            />
            {verdict.tone === "mute" && (
              <p className="sync-r2-verdict mute">
                The bucket&rsquo;s S3 API value — or the account endpoint from the token screen.
              </p>
            )}
            {verdict.tone === "ok" && <OkLine verdict={verdict} />}
            {revealed && !bucketOk && (
              <p className="sync-r2-verdict mute" aria-live="polite">
                Account <code>{shortAccount(verdict.accountId)}</code>
                {jurisdictionTail(verdict.jurisdiction)} — now name the bucket below.
              </p>
            )}
            {revealed && bucketOk && (
              <p className="sync-r2-verdict ok" aria-live="polite">
                Bucket <code>{bucket}</code> · account <code>{shortAccount(verdict.accountId)}</code>
                {jurisdictionTail(verdict.jurisdiction)}
              </p>
            )}
            {verdict.tone === "bad" && <BadLine />}
          </label>
          {revealed && (
            <label>
              <span className="sync-field-name">Bucket name</span>
              <input
                value={bucket}
                onChange={(event) => setBucket(event.target.value)}
                placeholder="codex-sync-backups"
                spellCheck={false}
                autoCapitalize="none"
                autoCorrect="off"
                autoComplete="off"
              />
            </label>
          )}
          {revealed && <span aria-hidden="true" />}
          <KeysPair accessKeyId={accessKeyId} onAccessKeyId={setAccessKeyId} secret={secret} onSecret={setSecret} />
        </div>
      </article>
      <CommitBar set={set} of={3} ready={set === 3 && !!name.trim()} />
    </div>
  );
}

/* ── C · trim only — the shipped coupling, less text ─────────────────────── */

function ModelTrim({ seed }: { seed: Seed }) {
  const [name, setName] = useState(seed.name);
  const [url, setUrl] = useState(seed.url ?? "");
  const [bucket, setBucket] = useState(seed.bucket ?? "");
  const [accountId, setAccountId] = useState(seed.accountId ?? "");
  const [jurisdiction, setJurisdiction] = useState<R2Jurisdiction>(seed.jurisdiction ?? "default");
  const [accessKeyId, setAccessKeyId] = useState(seed.accessKeyId ?? "");
  const [secret, setSecret] = useState(seed.secret ?? "");

  // The shipped coupling: a parsing paste fills bucket + account; editing
  // either part rewrites the URL (or blanks it while the parts are invalid).
  const onUrl = (value: string) => {
    setUrl(value);
    const parsed = parseR2S3ApiUrl(value.trim());
    if (parsed) {
      setBucket(parsed.bucket);
      setAccountId(parsed.accountId);
      setJurisdiction(parsed.jurisdiction);
    } else {
      setBucket("");
      setAccountId("");
    }
  };
  const recompose = (nextBucket: string, nextAccount: string) => {
    const endpoint = r2EndpointForAccount(nextAccount, jurisdiction);
    setUrl(endpoint && isCloudflareR2BucketName(nextBucket) ? `${endpoint}/${nextBucket}` : "");
  };
  const set = [
    !!url.trim(),
    !!bucket.trim(),
    !!accountId.trim(),
    !!accessKeyId.trim(),
    !!secret.trim(),
  ].filter(Boolean).length;
  const ready = !!parseR2S3ApiUrl(url.trim()) && !!accessKeyId.trim() && !!secret.trim() && !!name.trim();
  return (
    <div className="sync-storage-details-panel">
      <article className="sync-storage-card">
        <CardHeader name={name} onName={setName} />
        <GuideRow />
        <div className="sync-storage-fields sync-r2-fields">
          <label className="sync-r2-url-field">
            <span className="sync-field-name">S3 API URL</span>
            <input
              type="url"
              value={url}
              onChange={(event) => onUrl(event.target.value)}
              placeholder="https://<account-id>.r2.cloudflarestorage.com/<bucket>"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
          </label>
          <label>
            <span className="sync-field-name">Bucket name</span>
            <input
              value={bucket}
              onChange={(event) => { setBucket(event.target.value); recompose(event.target.value, accountId); }}
              placeholder="codex-sync-backups"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
          </label>
          <label>
            <span className="sync-field-name">Account ID</span>
            <input
              value={accountId}
              onChange={(event) => { setAccountId(event.target.value); recompose(bucket, event.target.value); }}
              placeholder="023e105f4ecef8ad9ca31a8372d0c353"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
          </label>
          <KeysPair accessKeyId={accessKeyId} onAccessKeyId={setAccessKeyId} secret={secret} onSecret={setSecret} />
        </div>
      </article>
      <CommitBar set={set} of={5} ready={ready} />
    </div>
  );
}

/* ── D · parts, no URL ───────────────────────────────────────────────────── */

function ModelParts({ seed }: { seed: Seed }) {
  const [name, setName] = useState(seed.name);
  const [bucket, setBucket] = useState(seed.bucket ?? "");
  const [accountId, setAccountId] = useState(seed.accountId ?? "");
  const [jurisdiction, setJurisdiction] = useState<R2Jurisdiction>(seed.jurisdiction ?? "default");
  const [accessKeyId, setAccessKeyId] = useState(seed.accessKeyId ?? "");
  const [secret, setSecret] = useState(seed.secret ?? "");
  const endpoint = r2EndpointForAccount(accountId, jurisdiction);
  const address = endpoint && isCloudflareR2BucketName(bucket) ? `${endpoint}/${bucket}` : "";
  const set = [
    !!bucket.trim(),
    !!accountId.trim(),
    !!accessKeyId.trim(),
    !!secret.trim(),
  ].filter(Boolean).length;
  return (
    <div className="sync-storage-details-panel">
      <article className="sync-storage-card">
        <CardHeader name={name} onName={setName} />
        <GuideRow />
        <div className="sync-storage-fields sync-r2-fields">
          <label>
            <span className="sync-field-name">Bucket name</span>
            <input
              value={bucket}
              onChange={(event) => setBucket(event.target.value)}
              placeholder="codex-sync-backups"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
            />
          </label>
          <label>
            <span className="sync-field-name">Account ID</span>
            <input
              value={accountId}
              onChange={(event) => setAccountId(event.target.value)}
              placeholder="023e105f4ecef8ad9ca31a8372d0c353"
              spellCheck={false}
              autoCapitalize="none"
              autoCorrect="off"
              autoComplete="off"
              aria-invalid={!!accountId.trim() && !isCloudflareR2AccountId(accountId) || undefined}
            />
          </label>
          <label className="sync-r2-jurisdiction">
            <span className="sync-field-name">Jurisdiction</span>
            <span className="sync-select-shell">
              <select
                value={jurisdiction}
                onChange={(event) => setJurisdiction(event.target.value as R2Jurisdiction)}
              >
                <option value="default">Default</option>
                <option value="eu">European Union</option>
                <option value="fedramp">FedRAMP</option>
              </select>
            </span>
          </label>
          <p className="sync-r2-verdict sync-r2-address-echo">
            {address
              ? <span className="ok">Address <code>{address}</code></span>
              : <span className="mute">Account ID and bucket name build the address.</span>}
          </p>
          <KeysPair accessKeyId={accessKeyId} onAccessKeyId={setAccessKeyId} secret={secret} onSecret={setSecret} />
        </div>
      </article>
      <CommitBar
        set={set}
        of={4}
        ready={!!address && !!accessKeyId.trim() && !!secret.trim() && !!name.trim()}
      />
    </div>
  );
}

function ModelUrl({ seed }: { seed: Seed }) {
  const draft = useR2Draft(seed);
  return <UrlCard draft={draft} />;
}

/* ── specimen harness ────────────────────────────────────────────────────── */

function Panel({ title, standard, note, children }: {
  title: string;
  standard: string;
  note: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mockup-panel">
      <h3>{title}</h3>
      <p className="mockup-standard">{standard}</p>
      <p>{note}</p>
      <div className="mockup-stage">{children}</div>
    </section>
  );
}

function SectionHead({ title, lede }: { title: string; lede: ReactNode }) {
  return (
    <div className="mockup-section">
      <h2>{title}</h2>
      <p>{lede}</p>
    </div>
  );
}

function Harness() {
  const [dark, setDark] = useState(false);
  const toggleTheme = () => {
    const next = !dark;
    setDark(next);
    document.documentElement.setAttribute("data-theme", next ? "dark" : "light");
  };
  return (
    <div className="app">
      <style>{SHELL_CSS}</style>
      <style>{PROPOSED_CSS}</style>
      <div className="mockup-shell">
        <header className="mockup-head">
          <div>
            <h1>R2 setup · redesign</h1>
            <p>
              The agent action is decided: no banner — an accent-tinted <em>Copy agent
              instruction</em> button in the guide row, unmissable without claiming the
              card&rsquo;s one primary. After the click its slot carries the real lifecycle:
              listening with a loader, then the connected note while the form fills itself
              (the CLI finish is faked seconds after the click, so all three states are on
              screen). One decision left: the field model below.
            </p>
          </div>
          <div className="mockup-controls">
            <button type="button" className="btn" onClick={toggleTheme}>
              <Icon name={dark ? "sun" : "moon"} size={13} /> {dark ? "Light" : "Dark"}
            </button>
          </div>
        </header>

        <SectionHead
          title="I · The agent action — decided"
          lede={<>Volume 3·2: the guide-row button in the accent&rsquo;s soft tint, the pair the
            storage icon already wears. The shipped listening mechanic, chrome only.</>}
        />

        <Panel
          title="The accent button"
          standard="Decided — brand volume in the guide row, no banner"
          note={<>The terminal glyph and the accent tint make the agent route the loudest thing
            in the row while <em>Create storage</em> keeps the card&rsquo;s only primary. Click
            it: the slot flips to the listening state, and when the CLI finishes (faked here
            seconds later) it becomes the connected note as the form below fills itself — the
            shipped adoption mechanic, drawn in place.</>}
        >
          <VolumeAccent />
        </Panel>

        <SectionHead
          title="II · The field model"
          lede={<>Parked from the last round, unchanged: what the first paste is. Cloudflare&rsquo;s
            two handouts, for judging: the bucket&rsquo;s Settings page copies
            <code> {EU_URL}</code>; the token screen shows both keys and, at its foot, the bare
            account endpoint <code>{BARE_ENDPOINT}</code>.</>}
        />

        <Panel
          title="A · One address"
          standard="Three fields — one string, parsed and echoed"
          note={<>The URL is the only address field; the judged line under it echoes bucket,
            account, and jurisdiction instead of asking for them. Fewest fields and one mental
            model, at one cost: the token screen&rsquo;s bare endpoint is answered with a
            correction — right host, add <code>/your-bucket</code> — rather than accepted.
            Clear the field to see the template placeholder and the where-to-find hint; paste
            garbage for the error that restates the shape.</>}
        >
          <div className="mockup-column">
            <ModelUrl seed={{ name: "R2 storage 2", url: EU_URL, accessKeyId: KEY_ID, secret: "mock-secret" }} />
          </div>
        </Panel>

        <Panel
          title="B · Address, then the missing part"
          standard="Both handouts accepted — a field appears for what's missing"
          note={<>Same three-count as A, but the bare account endpoint is progress, not error:
            the verdict echoes the account and asks for the one missing part, and a bucket
            field appears for it. Seeded here with the endpoint paste so the reveal is visible —
            type a bucket name and the verdict completes; paste the full bucket URL instead and
            the extra field folds away. Cost: the form changes shape while you type.</>}
        >
          <div className="mockup-column">
            <ModelReveal seed={{ name: "R2 storage 2", url: BARE_ENDPOINT, accessKeyId: KEY_ID, secret: "mock-secret" }} />
          </div>
        </Panel>

        <Panel
          title="C · Trim only"
          standard="The shipped five-field coupling, text diet applied"
          note={<>Nothing structural: URL, bucket, and account all stay, still coupled both ways
            — seeded here by one URL paste, which is why three of the five are already filled.
            Familiar and zero-risk, but the redundancy the trim was asked to remove is still on
            screen: three controls carrying one fact, and a counter that jumps by three on a
            paste. Edit the bucket or account and watch the URL rewrite itself.</>}
        >
          <div className="mockup-column">
            <ModelTrim seed={{ name: "R2 storage 2", url: EU_URL, bucket: "codex-sync-backups", accountId: ACCOUNT, jurisdiction: "eu", accessKeyId: KEY_ID, secret: "mock-secret" }} />
          </div>
        </Panel>

        <Panel
          title="D · Parts, no URL"
          standard="Manual parts, derived address — the AWS-console shape"
          note={<>No URL field: bucket, account ID, and an explicit jurisdiction select, with the
            derived address echoed underneath. No parsing magic and nothing hidden — but it is
            the most typing and the most hunting (the account ID lives on the dashboard
            sidebar, not next to the bucket), and jurisdiction becomes a control the user must
            know to set, where A–C read it from the pasted URL for free.</>}
        >
          <div className="mockup-column">
            <ModelParts seed={{ name: "R2 storage 2", bucket: "codex-sync-backups", accountId: ACCOUNT, jurisdiction: "eu", accessKeyId: KEY_ID, secret: "mock-secret" }} />
          </div>
        </Panel>
      </div>
    </div>
  );
}

/* ── proposed CSS — the only rules the redesign adds; everything else on the
      page renders through App.css and RetroTheme.css as shipped ──────────── */

const PROPOSED_CSS = `
/* The judged line under the URL: same scale and tones as the setup page's
   .sync-setup-path-hint, minus its grid-column (this one lives inside the
   field's own label). */
.sync-r2-verdict {
  margin: 0;
  color: var(--text-3);
  font-size: var(--type-2xs);
  line-height: 1.5;
}
.sync-r2-verdict.ok, .sync-r2-verdict .ok { color: var(--success-text); }
.sync-r2-verdict.warn { color: var(--warning-text); }
.sync-r2-verdict.bad { color: var(--danger-text); }
.sync-r2-verdict.mute, .sync-r2-verdict .mute { color: var(--text-3); }
.sync-r2-verdict code {
  font-family: var(--font-mono);
  font-size: var(--type-2xs);
  overflow-wrap: anywhere;
}

/* One caption for the credential pair; the negative margin tucks it under the
   12px row gap so it reads as the pair's footnote, not a third row. */
.sync-r2-keys-hint {
  grid-column: 1 / -1;
  margin: -6px 0 0;
  color: var(--text-3);
  font-size: var(--type-2xs);
  line-height: 1.5;
}

/* Model D only: the derived address, full row; and the jurisdiction select at
   the field-grid's control height. */
.sync-r2-address-echo { grid-column: 1 / -1; margin: -4px 0 0; }
.sync-r2-jurisdiction .sync-select-shell > select { height: 33px; }

/* The agent action in the guide row. The button sits at the row's control
   height; the accent variant borrows the storage icon's soft tint so it is
   branded without competing with the card's one primary. */
.sync-r2-guide-btn {
  min-height: 26px;
  padding: 2px 9px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex: none;
}
.sync-r2-agent-cta {
  color: var(--accent);
  background: var(--accent-soft);
  border-color: color-mix(in srgb, var(--accent) 32%, transparent);
}
.sync-r2-agent-cta:hover:not(:disabled) {
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, var(--accent-soft));
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}

/* The row tail after the click: listening, then connected. */
.sync-r2-guide-wait {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  flex: none;
  color: var(--text-2);
  font-size: var(--type-xs);
}
.sync-r2-guide-connected {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex: none;
  color: var(--success-text);
  font-size: var(--type-xs);
}

`;

const SHELL_CSS = `
/* The app is a fixed shell — html, body, #root and .app are all height-locked
   and overflow: hidden, because in Tauri only the inner panes scroll. A
   mockup is a document, so it has to hand the wheel back to the page. */
html, body, #root { height: auto; min-height: 100%; overflow: visible; }
.app { height: auto; min-height: 100vh; overflow: visible; }

/* width:100% is load-bearing — .app is a column flex container, and an auto
   cross-axis margin without it lets the shell shrink to its content. */
.mockup-shell { width: 100%; max-width: 1100px; margin: 0 auto; padding: 28px 24px 90px; }
.mockup-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 24px; margin-bottom: 8px; }
.mockup-head h1 { margin: 0 0 6px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-head p { max-width: 78ch; margin: 0; color: var(--text-2); font-size: var(--type-md); line-height: 1.55; }
.mockup-head code, .mockup-panel > p code, .mockup-section > p code { font-family: var(--font-mono); font-size: 0.92em; overflow-wrap: anywhere; }
.mockup-controls { display: inline-flex; gap: 8px; flex: 0 0 auto; }
.mockup-section { margin-top: 46px; padding-top: 24px; border-top: 2px solid var(--border-strong, var(--border)); }
.mockup-section > h2 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-xl); font-weight: 400; }
.mockup-section > p { max-width: 84ch; margin: 0; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }
.mockup-panel { margin-top: 34px; padding-top: 20px; border-top: 1px solid var(--border); }
.mockup-panel > h3 { margin: 0 0 4px; color: var(--text-0); font-family: var(--font-pixel); font-size: var(--type-lg); font-weight: 400; }
.mockup-panel > .mockup-standard { margin: 0 0 8px; color: var(--text-3); font-family: var(--font-mono); font-size: var(--type-2xs); letter-spacing: .04em; text-transform: uppercase; }
.mockup-panel > p { max-width: 84ch; margin: 0 0 14px; color: var(--text-2); font-size: var(--type-sm); line-height: 1.55; }

/* The stage plays the page: the card column sits on the shell tint at the
   width the storage settings page gives it. */
.mockup-stage {
  display: flex;
  justify-content: center;
  padding: 36px 24px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--app-shell);
}
.mockup-column { width: 100%; max-width: 860px; display: flex; flex-direction: column; gap: 10px; }
body { background: var(--app-shell); }
`;

document.documentElement.setAttribute("data-theme", "light");
// Reuse the root across hot updates: a second createRoot on the same node is
// a React error, and this file is edited while it is on screen.
const container = document.getElementById("root")! as HTMLElement & { _root?: Root };
container._root ??= createRoot(container);
container._root.render(<StrictMode><Harness /></StrictMode>);

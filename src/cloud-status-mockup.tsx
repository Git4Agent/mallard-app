import { StrictMode, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import Icon, { type IconName } from "./components/Icons";
import { NoticeZone, type NoticeItem } from "./components/notice/Notice";
import { PathBarMenu, type PathBarMenuGroup, type PathBarMenuItem } from "./components/sync-review/SyncPathBar";
import { cloudStatusNotice, cloudVerbs, cloudVersionUrl } from "./components/project-sync/managedCloudLink";
import type { ManagedCloudPublicationStatus, ManagedCloudReleaseState } from "./types";
import { writeClipboardText } from "./components/sync-review/clipboard";
import type { ManagedCloudLink, ManagedCloudStatus } from "./types";
import "./App.css";
import "./cloud-status-mockup.css";

/**
 * Mockup: one Mallard Cloud mark in the path bar that carries every state,
 * and a popover that holds the Cloud verbs (out of the project menu).
 *
 * Today's frames are the shipped pieces: the chip's markup class for class,
 * the real `PathBarMenu` held open, the real `NoticeZone`, and every word from
 * `releaseChip` / `cloudStatusChip` / `cloudStatusNotice` / `cloudVerbs`. The
 * proposal is `headline()` below (a candidate for managedCloudLink.ts), the
 * composite marks, and the popover.
 *
 * `npm run dev`, then /cloud-status-mockup.html (the choices:
 * `?dir=all|a|b|c&state=<id>&width=580|800|1100&mark=composite|swap&copy=split|links|rows&nonready=mark|notice&theme=light|dark`)
 * or /cloud-status-b-mockup.html (the picks: `?state=<id>&theme=light|dark`).
 */

/* ——— the chip as it shipped before this redesign (the "Today" frames) ——— */

function releaseChip(state: ManagedCloudReleaseState): { label: string; severity: "plain" | "warning" | "danger"; detail: string } {
  switch (state.kind) {
    case "not_pushed": return { label: "Not pushed", severity: "plain", detail: "Registered on Mallard Cloud; nothing has been pushed yet." };
    case "private": return { label: "Not requested", severity: "plain", detail: "The draft is not under review." };
    case "waiting_owner": return { label: "Pending approval", severity: "plain", detail: "Public release awaits Owner approval." };
    case "waiting_super_admin": return { label: "Pending approval", severity: "plain", detail: "Public release awaits Admin approval." };
    case "public": return { label: "Published", severity: "plain", detail: "The draft matches the published version." };
    case "snapshot_changed": return { label: "Not requested", severity: "plain", detail: "The current draft is not under review." };
    case "rejected": return { label: "Rejected", severity: "danger", detail: state.reason ? `The release request was rejected: ${state.reason}` : "The release request was rejected." };
  }
}

function cloudStatusChip(link: ManagedCloudLink, publication?: ManagedCloudPublicationStatus | null) {
  const review = releaseChip(link.release);
  const visibility = link.visibility === "public" ? publication?.public_revision != null ? `Public version ${publication.public_revision}` : "Public" : "Private";
  const state = publication?.state ?? "unavailable";
  const publicationLabel = { no_draft: "No draft yet", not_public: "Not public", matches_published: "Draft matches published version", unpublished_draft: "Unpublished draft", unavailable: "Publication status unavailable" }[state];
  const labels = [visibility];
  if (state === "unpublished_draft" || state === "unavailable") labels.push(publicationLabel);
  labels.push(review.label);
  const draft = publication?.private_revision != null ? `Draft version ${publication.private_revision}. ` : "";
  return {
    label: labels.join(" · "),
    severity: review.severity === "danger" ? "danger" as const : state === "unavailable" ? "warning" as const : "plain" as const,
    detail: `${visibility}. ${draft}${publicationLabel}. ${review.detail}`,
  };
}

/* ——— fixtures: the statuses the service can answer ———————————————— */

const LINK: ManagedCloudLink = {
  repository_id: "repo_one", visibility: "public", can_pull: true, can_push: true,
  website_url: "https://mallard-ai.com/rikaqu0223-arch/HorrorGame", owner_login: "rikaqu0223-arch", name: "HorrorGame",
  repository_revision: 7, can_request_release: true, role: "owner", release: { kind: "public" },
};
const REPO = `${LINK.owner_login}/${LINK.name}`;
const STORAGE = "Mallard Cloud Rika321";

const ready = (
  of: Partial<ManagedCloudLink>,
  publication: ManagedCloudStatus["publication"],
): ManagedCloudStatus => ({ repository_id: "repo_one", state: "ready", message: null, link: { ...LINK, ...of }, publication });
const failed = (state: ManagedCloudStatus["state"], message: string): ManagedCloudStatus => (
  { repository_id: "repo_one", state, link: null, message }
);

interface Case { id: string; name: string; status: ManagedCloudStatus }

const CASES: Case[] = [
  { id: "not-pushed", name: "Registered, nothing pushed", status: ready({ visibility: "private", release: { kind: "not_pushed" }, can_request_release: false }, { state: "no_draft", private_revision: null, public_revision: null }) },
  { id: "private", name: "Private, draft v5", status: ready({ visibility: "private", release: { kind: "private" } }, { state: "not_public", private_revision: 5, public_revision: null }) },
  { id: "published", name: "Public, draft is public v3", status: ready({ release: { kind: "public" }, can_request_release: false }, { state: "matches_published", private_revision: 3, public_revision: 3 }) },
  { id: "new-draft", name: "Public v3, newer draft v5", status: ready({ release: { kind: "snapshot_changed" } }, { state: "unpublished_draft", private_revision: 5, public_revision: 3 }) },
  { id: "contributor", name: "Public v3, you contribute", status: ready({ release: { kind: "snapshot_changed" }, role: null, can_request_release: false }, { state: "unpublished_draft", private_revision: 6, public_revision: 3 }) },
  { id: "pending-owner", name: "Release asked, Owner decides", status: ready({ release: { kind: "waiting_owner", requested_by: "maya" }, can_request_release: false }, { state: "unpublished_draft", private_revision: 5, public_revision: 3 }) },
  { id: "pending-admin", name: "Release asked, Admin decides", status: ready({ release: { kind: "waiting_super_admin" }, can_request_release: false }, { state: "unpublished_draft", private_revision: 5, public_revision: 3 }) },
  { id: "rejected", name: "Release rejected", status: ready({ release: { kind: "rejected", reason: "Session 12 holds an API key." } }, { state: "unpublished_draft", private_revision: 5, public_revision: 3 }) },
  { id: "no-compare", name: "Public, comparison unavailable", status: ready({ release: { kind: "public" }, can_request_release: false }, { state: "unavailable", private_revision: null, public_revision: 3 }) },
  { id: "sign-in", name: "Signed out", status: failed("sign_in_required", "Sign in to Mallard Cloud again.") },
  { id: "refused", name: "Account refused", status: failed("inaccessible", "Mallard Cloud refused this account. It may be suspended.") },
  { id: "offline", name: "Service unreachable", status: failed("unavailable", "Could not reach Mallard Cloud.") },
  { id: "deleting", name: "Repository being deleted", status: failed("deleting", "Push creates it again.") },
  { id: "gone", name: "Repository gone", status: failed("gone", "Deleted, or this account lost access. Push creates it again.") },
];

/* ——— the proposal: one headline per status ———————————————————————— */

type Mark = "cloud" | "check" | "dot" | "clock" | "lock" | "x" | "alert" | "user" | "off";
type Tone = "plain" | "success" | "info" | "warning" | "danger";
interface Headline { rank: number; word: string; line: string; mark: Mark; tone: Tone }

/** The one state the mark shows, most urgent first. A candidate for
 *  managedCloudLink.ts beside `cloudVerbs`: the chip, its accessible name and
 *  the popover's head all read it, so the words live in one place. */
function headline(status: ManagedCloudStatus): Headline {
  if (status.state === "sign_in_required") return { rank: 1, word: "Sign in", line: "Signed out of Mallard Cloud.", mark: "user", tone: "warning" };
  if (status.state === "inaccessible") return { rank: 2, word: "No access", line: status.message ?? "Mallard Cloud refused this account.", mark: "alert", tone: "warning" };
  if (status.state === "unavailable") return { rank: 2, word: "Can't check", line: status.message ?? "Could not reach Mallard Cloud.", mark: "alert", tone: "warning" };
  if (status.state === "deleting") return { rank: 3, word: "Being deleted", line: "Push creates it again.", mark: "off", tone: "warning" };
  if (status.state === "gone") return { rank: 3, word: "Gone", line: "Push creates it again.", mark: "off", tone: "warning" };
  const link = status.link!;
  const pub = status.publication;
  const draft = pub?.private_revision != null ? `v${pub.private_revision}` : null;
  const live = pub?.public_revision != null ? `v${pub.public_revision}` : null;
  switch (link.release.kind) {
    case "rejected":
      return { rank: 4, word: "Rejected", line: link.release.reason ?? "The release was rejected.", mark: "x", tone: "danger" };
    case "waiting_owner":
      return { rank: 5, word: "In review", line: `Waiting for the Owner. ${link.release.requested_by} asked.`, mark: "clock", tone: "info" };
    case "waiting_super_admin":
      return { rank: 5, word: "In review", line: "Waiting for a Mallard admin.", mark: "clock", tone: "info" };
    case "not_pushed":
      return { rank: 9, word: "Not pushed", line: "Nothing pushed yet.", mark: "cloud", tone: "plain" };
    default:
      break;
  }
  if (!pub || pub.state === "unavailable") return { rank: 2, word: "Can't compare", line: "Couldn't compare the draft with the public version.", mark: "alert", tone: "warning" };
  if (link.visibility === "private") return { rank: 8, word: "Private", line: draft ? `Draft ${draft}. Collaborators only.` : "Collaborators only.", mark: "lock", tone: "plain" };
  if (pub.state === "unpublished_draft") return { rank: 6, word: `Draft ${draft}`, line: `Newer than public ${live}. Not in review.`, mark: "dot", tone: "plain" };
  return { rank: 7, word: `Published ${live}`, line: "The draft is the public version.", mark: "check", tone: "success" };
}

/* ——— composite marks: the shipped cloud, bitten by a badge ——————————— */

// The shipped `cloud` path (Icons.tsx), 24-unit grid.
const CLOUD_D = "M22 12v-1h-2V9h-1V8h-1V7h-3v1h-1V6h-1V5H8v1H7v1H6v1H5v3H2v1H1v5h1v1h1v1h2v1h14v-1h2v-1h1v-1h1v-5h-1Zm0 4h-1v1h-1v1h-2v1H6v-1H4v-1H3v-1H2v-3h1v-1h3V9h1V8h1V7h1V6h3v1h1v2h3V8h1v1h1v1h1v2h2v1h1v3Z";

/* Badges on a 12×12 field at x 12–23, y 12–23: '#' is ink, 'o' a knockout,
   '.' outside the disc. One-unit steps, two-unit strokes. */
const BADGE: Record<Exclude<Mark, "cloud" | "off">, string[]> = {
  dot: [
    "...######...", ".##########.", ".##########.", "############", "############", "############",
    "############", "############", "############", ".##########.", ".##########.", "...######...",
  ],
  check: [
    "...######...", ".##########.", ".##########.", "########oo##", "#######ooo##", "##oo##ooo###",
    "##oooooo####", "###oooo#####", "####oo######", ".##########.", ".##########.", "...######...",
  ],
  clock: [
    "...######...", ".##########.", ".####oo####.", "#####oo#####", "#####oo#####", "#####oooo###",
    "#####oooo###", "############", "############", ".##########.", ".##########.", "...######...",
  ],
  x: [
    "...######...", ".##########.", ".##########.", "##oo####oo##", "###oo##oo###", "####oooo####",
    "####oooo####", "###oo##oo###", "##oo####oo##", ".##########.", ".##########.", "...######...",
  ],
  alert: [
    "...######...", ".##########.", ".####oo####.", "#####oo#####", "#####oo#####", "#####oo#####",
    "############", "#####oo#####", "#####oo#####", ".##########.", ".##########.", "...######...",
  ],
  user: [
    "...######...", ".##########.", ".####oo####.", "####oooo####", "####oooo####", "#####oo#####",
    "############", "###oooooo###", "##oooooooo##", ".##########.", ".##########.", "...######...",
  ],
  lock: [
    "...######...", ".##########.", ".###oooo###.", "###oo##oo###", "###oo##oo###", "##oooooooo##",
    "##oooooooo##", "##oooooooo##", "##oooooooo##", ".##########.", ".##########.", "...######...",
  ],
};

function runs(cells: (x: number, y: number) => boolean, x0: number, y0: number, w: number, h: number): string {
  let d = "";
  for (let y = 0; y < h; y += 1) {
    let x = 0;
    while (x < w) {
      if (!cells(x, y)) { x += 1; continue; }
      const start = x;
      while (x < w && cells(x, y)) x += 1;
      d += `M${x0 + start} ${y0 + y}h${x - start}v1h-${x - start}Z`;
    }
  }
  return d;
}

const badgeInk = (rows: string[]) => runs((x, y) => rows[y][x] === "#", 12, 12, 12, 12);
const inDisc = (x: number, y: number) => x >= 0 && y >= 0 && x < 12 && y < 12 && BADGE.dot[y][x] === "#";
// The disc, grown one unit every way: the gap between the badge and the cloud.
const HALO_D = runs((x, y) => {
  for (let dy = -1; dy <= 1; dy += 1) for (let dx = -1; dx <= 1; dx += 1) if (inDisc(x - 1 + dx, y - 1 + dy)) return true;
  return false;
}, 11, 11, 14, 14);
// The slash of "off": two-unit steps from top left to bottom right, and its gap.
const SLASH_D = Array.from({ length: 19 }, (_, k) => `M${k + 2} ${k + 2}h2v2h-2Z`).join("");
const SLASH_HALO_D = Array.from({ length: 19 }, (_, k) => `M${k + 1} ${k + 1}h4v4h-4Z`).join("");

const TONE_INK: Record<Tone, string> = {
  plain: "currentColor", success: "var(--success-text)", info: "var(--info-text)", warning: "var(--warning-text)", danger: "var(--danger-text)",
};

const SWAP: Record<Mark, IconName> = {
  cloud: "cloud", check: "check-circle", dot: "git-commit", clock: "clock", lock: "lock", x: "octagon-x", alert: "alert-triangle", user: "user", off: "link-off",
};

function MarkGlyph({ mark, tone, size, style }: { mark: Mark; tone: Tone; size: number; style: "composite" | "swap" }) {
  const raw = useId();
  const id = `m${raw.replace(/[^a-zA-Z0-9_-]/g, "")}`;
  if (style === "swap" || mark === "cloud") {
    return <span className="spec-swap" style={{ color: mark === "cloud" ? undefined : TONE_INK[tone] }}><Icon name={SWAP[mark]} size={size} /></span>;
  }
  const halo = mark === "off" ? SLASH_HALO_D : HALO_D;
  return (
    <svg aria-hidden="true" data-mark={mark} focusable="false" width={size} height={size} viewBox="0 0 24 24"
      shapeRendering={size % 24 === 0 ? "crispEdges" : "geometricPrecision"}>
      <defs>
        <mask id={id} maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">
          <rect width="24" height="24" fill="white" />
          <path d={halo} fill="black" />
        </mask>
      </defs>
      <path d={CLOUD_D} mask={`url(#${id})`} style={{ fill: "currentColor" }} />
      <path d={mark === "off" ? SLASH_D : badgeInk(BADGE[mark])} style={{ fill: TONE_INK[tone] }} />
    </svg>
  );
}

/* Ink, as the icon-set note measures it: rasterize on the 24-unit grid and
   sample one point per coarse cell (what size 12 paints). */
function inkOf(mark: Mark): number {
  const canvas = document.createElement("canvas");
  canvas.width = 24; canvas.height = 24;
  const g = canvas.getContext("2d")!;
  g.fillStyle = "#000";
  g.fill(new Path2D(CLOUD_D));
  if (mark !== "cloud") {
    g.globalCompositeOperation = "destination-out";
    g.fill(new Path2D(mark === "off" ? SLASH_HALO_D : HALO_D));
    g.globalCompositeOperation = "source-over";
    g.fill(new Path2D(mark === "off" ? SLASH_D : badgeInk(BADGE[mark])));
  }
  const px = g.getImageData(0, 0, 24, 24).data;
  let ink = 0;
  for (let cy = 0; cy < 12; cy += 1) for (let cx = 0; cx < 12; cx += 1) if (px[((cy * 2 + 1) * 24 + cx * 2 + 1) * 4 + 3] > 127) ink += 1;
  return ink;
}

/* What a screen paints at `size` css px and `dpr`: the mark rasterized with
   antialiasing onto that many device pixels, then blown up without smoothing. */
function Raster({ mark, size, dpr }: { mark: Mark; size: number; dpr: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const n = size * dpr;
    canvas.width = n; canvas.height = n;
    const g = canvas.getContext("2d")!;
    const ink = getComputedStyle(canvas).color;
    g.clearRect(0, 0, n, n);
    g.save();
    g.scale(n / 24, n / 24);
    g.fillStyle = ink;
    g.fill(new Path2D(CLOUD_D));
    if (mark !== "cloud") {
      g.globalCompositeOperation = "destination-out";
      g.fill(new Path2D(mark === "off" ? SLASH_HALO_D : HALO_D));
      g.globalCompositeOperation = "source-over";
      g.fill(new Path2D(mark === "off" ? SLASH_D : badgeInk(BADGE[mark])));
    }
    g.restore();
  }, [mark, size, dpr]);
  return <canvas ref={ref} className="spec-raster" style={{ width: 64, height: 64 }} title={`${size}px at ${dpr}×`} />;
}

/* ——— knobs ——————————————————————————————————————————————————— */

type Dir = "all" | "a" | "b" | "c";
interface Knobs { dir: Dir; state: string; width: "580" | "800" | "1100"; mark: "composite" | "swap"; copy: CopyStyle; nonready: "mark" | "notice"; theme: "light" | "dark" }

function readKnobs(): Knobs {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T) => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    dir: pick("dir", ["all", "a", "b", "c"] as const, "all"),
    state: pick("state", CASES.map((item) => item.id), "new-draft"),
    width: pick("width", ["580", "800", "1100"] as const, "1100"),
    mark: pick("mark", ["composite", "swap"] as const, "composite"),
    copy: pick("copy", ["split", "links", "rows"] as const, "split"),
    nonready: pick("nonready", ["mark", "notice"] as const, "mark"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const noop = () => {};

/* ——— the shipped project menu, and the one after ————————————————————— */

function projectMenu(withCloud: boolean): PathBarMenuGroup[] {
  const cloud: PathBarMenuItem[] = withCloud ? [
    { id: "open-managed-cloud", label: "View draft", detail: REPO, detailIsPath: true, icon: "external-link", onSelect: noop },
    { id: "view-published-cloud", label: "View published version", icon: "external-link", onSelect: noop },
  ] : [];
  return [
    {
      items: [
        { id: "reveal", label: "Reveal in Finder", icon: "folder", onSelect: noop },
        { id: "agent", label: "Agent profile…", detail: "Default Claude · Default Codex", icon: "terminal", onSelect: noop },
        ...cloud,
        { id: "rename", label: "Rename project…", icon: "edit", onSelect: noop },
        { id: "delete-project-sessions", label: "Delete all local sessions…", icon: "trash", danger: true, onSelect: noop },
      ],
    },
    {
      label: "Switch to",
      items: [
        { id: "demo", label: "mallard_test_demo", detail: "~/Desktop/project/mallard_test_demo", detailIsPath: true, icon: "git-folder", onSelect: noop },
        { id: "echo", label: "echo-park-horror", detail: "~/Desktop/project/echo-park-horror", detailIsPath: true, icon: "git-folder", onSelect: noop },
      ],
    },
    { items: [{ id: "add-project", label: "Add project…", icon: "plus", onSelect: noop }] },
  ];
}

/* ——— the bar ——————————————————————————————————————————————————— */

function Bar({ menu, chip }: { menu?: PathBarMenuGroup[]; chip: ReactNode }) {
  return (
    <header className="sync-pathbar sync-titlebar">
      <ol className="sync-path" aria-label="HorrorGame path">
        <li className="sync-path-item">
          <button type="button" className="sync-path-crumb root">
            <Icon name="storage-mallard" size={14} />
            <span>{STORAGE}</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
        </li>
        <li className="sync-path-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <button type="button" className="sync-path-crumb" aria-haspopup="menu" aria-expanded={!!menu}>
            <Icon name="git-folder" size={14} />
            <span>HorrorGame</span>
            <Icon name="chevron-down" size={11} className="sync-path-chevron" />
          </button>
          {menu && (
            <PathBarMenu
              id={`spec-project-menu-${menu.length}-${menu[0].items.length}`}
              groups={menu}
              header={{ label: "HorrorGame", detail: "~/Desktop/project/HorrorGame", detailIsPath: true, icon: "git-folder" }}
              label="Project"
              align="start"
              open
              registerRef={noop}
              onClose={noop}
            />
          )}
        </li>
        <li className="sync-path-item leaf-item">
          <span className="sync-path-separator" aria-hidden="true">/</span>
          <span className="sync-path-crumb section leaf" aria-current="page"><span>Git history</span></span>
        </li>
      </ol>
      <div className="sync-path-actions">
        {chip}
        <button type="button" className="sync-path-icon-action" title="Refresh activity" aria-label="Refresh activity">
          <span aria-hidden="true" className="icon-spin-frame"><Icon name="refresh" size={16} /></span>
        </button>
        <div className="sync-path-action-group" role="group" aria-label="Sync actions">
          <button type="button" className="sync-path-sync"><Icon name="download" size={14} /><span className="sync-path-sync-label">Pull</span><span className="sync-path-count">12</span></button>
          <button type="button" className="sync-path-sync"><Icon name="upload" size={14} /><span className="sync-path-sync-label">Push</span><span className="sync-path-count">40</span></button>
        </div>
        <span className="sync-path-divider" aria-hidden="true" />
        <div className="sync-path-overflow">
          <button type="button" className="sync-path-more" title="App actions" aria-label="App actions"><Icon name="more" size={16} /></button>
        </div>
      </div>
    </header>
  );
}

/** Today's chip, as ProjectLinksWorkspace renders it: only for a ready link. */
function TodayChip({ status }: { status: ManagedCloudStatus }) {
  if (status.state !== "ready" || !status.link) return null;
  const chip = cloudStatusChip(status.link, status.publication);
  return (
    <span
      className={`sync-path-attention sync-path-cloud-status${chip.severity === "plain" ? "" : ` ${chip.severity}`}`}
      tabIndex={0}
      data-tooltip={`Mallard Cloud: ${chip.detail}`}
      aria-label={`Mallard Cloud: ${chip.label}. ${chip.detail}`}
    >
      <Icon name="cloud" size={13} />
      <b>{releaseChip(status.link.release).label}</b>
    </span>
  );
}

function WorkspaceNote({ status }: { status: ManagedCloudStatus }) {
  const note = cloudStatusNotice(status);
  if (!note) return null;
  const item: NoticeItem = {
    key: "managed-cloud-status", severity: "warning", title: note.title, message: note.message,
    action: note.action === null ? undefined : (
      <button type="button" className="btn btn-ghost">{note.action === "storage_settings" ? "Storage settings" : "Retry"}</button>
    ),
  };
  return <NoticeZone items={[item]} subject="notes about this workspace" timing="standing" label="Notes about this workspace" />;
}

/* ——— the proposal: the chip as a button, and its popover ————————————— */

type Face = "mark" | "word";

function CloudButton({ status, face, markStyle, open, tip, children }: {
  status: ManagedCloudStatus; face: Face; markStyle: "composite" | "swap"; open: boolean; tip?: boolean; children?: ReactNode;
}) {
  const head = headline(status);
  const severity = head.tone === "warning" || head.tone === "danger" ? ` ${head.tone}` : "";
  return (
    <span className="spec-anchor">
      <button
        type="button"
        className={`sync-path-attention sync-path-cloud-status spec-cloud-button ${face}${severity}${tip ? " spec-tip-on" : ""}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Mallard Cloud: ${head.word}. ${head.line}`}
        data-tooltip={`${head.word}. ${head.line}`}
        data-rank={head.rank}
      >
        <MarkGlyph mark={head.mark} tone={head.tone} size={16} style={markStyle} />
        {face === "word" && <b>{head.word}</b>}
        {face === "word" && <Icon name="chevron-down" size={11} className="sync-path-chevron" />}
      </button>
      {open && children}
    </span>
  );
}

type CopyStyle = "split" | "links" | "rows";

interface CloudHref { name: string; short: string; url: string }
interface Row { id: string; label: string; icon: IconName; danger?: boolean; primary?: boolean; href?: CloudHref }

/** The Release tab on the website, where the Owner or an Admin decides and
 *  where a request's answer is read. Proposed beside `cloudVersionUrl`. */
function cloudReleaseUrl(link: ManagedCloudLink): string {
  const url = new URL(link.website_url);
  url.searchParams.set("tab", "release");
  return url.toString();
}

function cloudRows(status: ManagedCloudStatus): Row[] {
  const verbs = cloudVerbs(status);
  const link = status.link;
  const rows: Row[] = [];
  if (!link) return rows;
  const release: CloudHref = { name: "release link", short: "Release", url: cloudReleaseUrl(link) };
  const asked = link.release.kind === "waiting_owner" || link.release.kind === "waiting_super_admin" || link.release.kind === "rejected";
  /* One row carries the release link in every state: the request itself
     while it can be sent, the open request while it waits or after it was
     turned down. A contributor has no Release tab to send. */
  if (verbs.requestRelease) rows.push({ id: "release", label: "Request public release…", icon: "check-circle", primary: true, href: release });
  else if (asked && link.role !== null) rows.push({ id: "release", label: "View release request", icon: "external-link", href: release });
  if (verbs.viewDraft) rows.push({ id: "draft", label: "View draft", icon: "external-link", href: { name: "draft link", short: "Draft", url: cloudVersionUrl(link, "private") } });
  if (verbs.viewPublished) rows.push({ id: "public", label: "View published version", icon: "external-link", href: { name: "published link", short: "Public", url: cloudVersionUrl(link, "public") } });
  if (verbs.discardDraft) rows.push({ id: "discard", label: "Discard draft…", icon: "x", danger: true });
  return rows;
}

/** Copy, then say so for two seconds, as File info's rows do. */
function useCopier(frozen?: string) {
  const [copied, setCopied] = useState<string | null>(frozen ?? null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!copied || copied === frozen) return;
    const timeout = window.setTimeout(() => setCopied(null), 2_000);
    return () => window.clearTimeout(timeout);
  }, [copied]);
  const copy = async (href: CloudHref) => {
    try {
      await writeClipboardText(href.url);
      setFailed(false);
      setCopied(href.name);
    } catch {
      setCopied(null);
      setFailed(true);
    }
  };
  return { copied, failed, copy };
}
type Copier = ReturnType<typeof useCopier>;

function CopyStatus({ copier }: { copier: Copier }) {
  return (
    <>
      <span className="spec-sr" role="status" aria-live="polite">
        {copier.failed ? "Couldn't copy the link." : copier.copied ? `Copied the ${copier.copied}.` : ""}
      </span>
      {copier.failed && <p className="spec-pop-line spec-copy-error" aria-hidden="true">Couldn't copy. The address is in the button's tooltip.</p>}
    </>
  );
}

/** The trailing half of a split row: copies what the row opens. */
function CopyButton({ href, copier }: { href: CloudHref; copier: Copier }) {
  const done = copier.copied === href.name;
  return (
    <button
      type="button"
      className={`spec-copy${done ? " copied" : ""}`}
      aria-label={`Copy ${href.name}`}
      title={`Copy ${href.name}\n${href.url}`}
      onClick={() => void copier.copy(href)}
    >
      {done && <small>Copied</small>}
      <Icon name={done ? "check" : "copy"} size={13} />
    </button>
  );
}

function RowButton({ row, menu }: { row: Row; menu: boolean }) {
  return (
    <button type="button" role={menu ? "menuitem" : undefined} tabIndex={menu ? -1 : undefined} className={`sync-path-menu-item${row.danger ? " danger" : ""}`}>
      <Icon name={row.icon} size={14} />
      <span className="sync-path-menu-copy"><span>{row.label}</span></span>
    </button>
  );
}

/** "rows": copying is a verb of its own, one more menu row under the one it copies. */
function CopyRow({ href, copier }: { href: CloudHref; copier: Copier }) {
  const done = copier.copied === href.name;
  return (
    <button type="button" role="menuitem" tabIndex={-1} className={`sync-path-menu-item spec-copy-row${done ? " copied" : ""}`} title={href.url} onClick={() => void copier.copy(href)}>
      <Icon name={done ? "check" : "copy"} size={14} />
      <span className="sync-path-menu-copy"><span>{done ? "Copied" : `Copy ${href.name}`}</span></span>
    </button>
  );
}

/** The verbs, drawn per copy style. A split row is two buttons, which a
 *  `role="menu"` can't hold, so it is a plain group the Tab key walks. */
function RowList({ rows, withPrimary, copy, copier, between }: { rows: Row[]; withPrimary: boolean; copy: CopyStyle; copier: Copier; between?: ReactNode }) {
  const shown = withPrimary ? rows : rows.filter((row) => !row.primary);
  const plain = shown.filter((row) => !row.danger);
  const danger = shown.filter((row) => row.danger);
  if (shown.length === 0) return null;
  /* Only plain rows make a menu: a split row or a printed address is not a
     menuitem, so those styles are a group the Tab key walks. */
  const menu = copy === "rows";
  const draw = (row: Row): ReactNode[] => {
    if (copy === "split" && row.href) {
      return [
        <div key={row.id} className="spec-link-row">
          <RowButton row={row} menu={false} />
          <CopyButton href={row.href} copier={copier} />
        </div>,
      ];
    }
    const out: ReactNode[] = [<RowButton key={row.id} row={row} menu={menu} />];
    if (copy === "rows" && row.href) out.push(<CopyRow key={`${row.id}-copy`} href={row.href} copier={copier} />);
    return out;
  };
  return (
    <div className="sync-path-menu-list" role={menu ? "menu" : "group"} aria-label="Mallard Cloud">
      {plain.flatMap(draw)}
      {between}
      {danger.length > 0 && plain.length > 0 && <div className="sync-path-menu-separator" role="separator" />}
      {danger.flatMap(draw)}
    </div>
  );
}

/** "links": every address printed, as File info prints a path; the row copies. */
function LinkFields({ rows, copier }: { rows: Row[]; copier: Copier }) {
  const hrefs = rows.flatMap((row) => row.href ? [row.href] : []);
  if (hrefs.length === 0) return null;
  return (
    <div className="spec-link-fields">
      <div className="sync-path-menu-label">Links</div>
      {hrefs.map((href) => (
        <button key={href.name} type="button" className="sync-history-debug-copy" title={`Copy ${href.name}`} onClick={() => void copier.copy(href)}>
          <span>{href.short}</span>
          <code>{href.url.replace(/^https:\/\//, "")}</code>
          <small className={copier.copied === href.name ? "copied" : undefined}>{copier.copied === href.name ? "Copied" : "Copy"}</small>
        </button>
      ))}
    </div>
  );
}

function facts(status: ManagedCloudStatus): [string, string][] {
  const link = status.link;
  if (!link) return [];
  const pub = status.publication;
  const role = link.role === "owner" ? "Owner" : link.role ? link.role[0].toUpperCase() + link.role.slice(1) : "Contributor";
  return [
    ["Draft", pub?.private_revision != null ? `v${pub.private_revision}` : "—"],
    ["Public", link.visibility === "private" ? "Private" : pub?.public_revision != null ? `v${pub.public_revision}` : "—"],
    ["Review", reviewWord(link)],
    ["You", role],
  ];
}

/** Where the release request stands, in the popover's words. */
function reviewWord(link: ManagedCloudLink): string {
  switch (link.release.kind) {
    case "not_pushed": return "—";
    case "private": case "snapshot_changed": return "Not asked";
    case "waiting_owner": return "Owner's turn";
    case "waiting_super_admin": return "Admin's turn";
    case "public": return "Approved";
    case "rejected": return "Rejected";
  }
}

function PopHead({ status, markStyle }: { status: ManagedCloudStatus; markStyle: "composite" | "swap" }) {
  const head = headline(status);
  return (
    <div className={`spec-pop-head ${head.tone}`}>
      <MarkGlyph mark={head.mark} tone={head.tone} size={24} style={markStyle} />
      <span className="sync-path-menu-copy">
        <span>{head.word}</span>
        <small>{status.link ? REPO : STORAGE}</small>
      </span>
    </div>
  );
}

/** For a status with no link: the notice's sentence and its one control. */
function NotReady({ status }: { status: ManagedCloudStatus }) {
  const note = cloudStatusNotice(status);
  if (!note) return null;
  return (
    <div className="spec-pop-body">
      <p className="spec-pop-line">{headline(status).line}</p>
      {note.action && (
        <button type="button" className="btn btn-ghost spec-pop-action">{note.action === "storage_settings" ? "Storage settings" : "Retry"}</button>
      )}
    </div>
  );
}

function ListPopover({ status, markStyle, copy, frozenCopied }: {
  status: ManagedCloudStatus; markStyle: "composite" | "swap"; copy: CopyStyle; frozenCopied?: string;
}) {
  const rows = cloudRows(status);
  const factRows = facts(status);
  const copier = useCopier(frozenCopied);
  const contributor = !!status.link && status.link.role === null && status.link.visibility === "public" && status.link.can_push;
  return (
    <div className={`sync-path-menu end spec-cloud-pop ${copy}`} role="dialog" aria-label="Mallard Cloud">
      <PopHead status={status} markStyle={markStyle} />
      {status.link ? (
        <>
          <p className="spec-pop-line">{headline(status).line}</p>
          <dl className="spec-pop-facts">
            {factRows.map(([term, value]) => <div key={term}><dt>{term}</dt><dd>{value}</dd></div>)}
          </dl>
          {contributor && <p className="spec-pop-line spec-pop-standing">Not a collaborator: anyone signed in can push here, and the Owner decides what's released.</p>}
          {rows.length > 0 && <div className="sync-path-menu-separator" role="separator" />}
          <RowList rows={rows} withPrimary copy={copy} copier={copier} between={copy === "links" && <LinkFields rows={rows} copier={copier} />} />
          <CopyStatus copier={copier} />
        </>
      ) : <NotReady status={status} />}
    </div>
  );
}

type Stop = { label: string; value: string; state: "done" | "current" | "todo" | "fail" };

function track(status: ManagedCloudStatus): Stop[] {
  const link = status.link!;
  const pub = status.publication;
  const draft = pub?.private_revision != null ? `v${pub.private_revision}` : "—";
  const live = link.visibility === "private" ? "Private" : pub?.public_revision != null ? `v${pub.public_revision}` : "—";
  const kind = link.release.kind;
  if (kind === "not_pushed") return [
    { label: "Draft", value: "—", state: "current" }, { label: "Review", value: "—", state: "todo" }, { label: "Public", value: live, state: "todo" },
  ];
  const reviewValue = reviewWord(link);
  const reviewState: Stop["state"] = kind === "rejected" ? "fail" : kind === "waiting_owner" || kind === "waiting_super_admin" ? "current" : kind === "public" ? "done" : "todo";
  const matches = pub?.state === "matches_published";
  return [
    { label: "Draft", value: draft, state: matches || reviewState !== "todo" ? "done" : "current" },
    { label: "Review", value: reviewValue, state: reviewState },
    { label: "Public", value: live, state: matches ? "current" : link.visibility === "private" ? "todo" : "done" },
  ];
}

function TrackPopover({ status, markStyle, copy }: { status: ManagedCloudStatus; markStyle: "composite" | "swap"; copy: CopyStyle }) {
  const rows = cloudRows(status);
  const primary = rows.find((row) => row.primary);
  const copier = useCopier();
  return (
    <div className={`sync-path-menu end spec-cloud-pop ${copy}`} role="dialog" aria-label="Mallard Cloud">
      <PopHead status={status} markStyle={markStyle} />
      {status.link ? (
        <>
          <ol className="spec-track" aria-label="Release">
            {track(status).map((stop) => (
              <li key={stop.label} className={stop.state}>
                <span className="spec-track-dot" aria-hidden="true">
                  {stop.state === "done" && <Icon name="check" size={10} />}
                  {stop.state === "fail" && <Icon name="x" size={10} />}
                </span>
                <b>{stop.label}</b>
                <small>{stop.value}</small>
              </li>
            ))}
          </ol>
          <p className="spec-pop-line">{headline(status).line}</p>
          {primary && (
            <div className="spec-primary-row">
              <button type="button" className="btn spec-pop-primary">{primary.label.replace(/…$/, "")}</button>
              {copy === "split" && primary.href && <CopyButton href={primary.href} copier={copier} />}
            </div>
          )}
          {rows.some((row) => !row.primary) && <div className="sync-path-menu-separator" role="separator" />}
          {copy === "rows" && primary?.href && (
            <div className="sync-path-menu-list" role="menu" aria-label="Release link"><CopyRow href={primary.href} copier={copier} /></div>
          )}
          <RowList rows={rows} withPrimary={false} copy={copy} copier={copier} between={copy === "links" && <LinkFields rows={rows} copier={copier} />} />
          <CopyStatus copier={copier} />
        </>
      ) : <NotReady status={status} />}
    </div>
  );
}

/* ——— frames —————————————————————————————————————————————————— */

function Frame({ title, note, pick, width, measure, children }: {
  title: string; note: ReactNode; pick?: boolean; width?: string; measure?: boolean; children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [chip, setChip] = useState<string | null>(null);
  useEffect(() => {
    if (!measure) return;
    let live = true;
    const read = () => {
      const node = ref.current?.querySelector<HTMLElement>(".sync-path-cloud-status");
      if (!live) return;
      setChip(node ? `${Math.round(node.getBoundingClientRect().width)}px` : "none");
    };
    void document.fonts.ready.then(() => setTimeout(read, 60));
    return () => { live = false; };
  });
  return (
    <div className="spec-frame-wrap">
      <h3 className="spec-col-title">{title}{pick && <em>recommended</em>}{measure && chip && <span className="spec-measure">Cloud chip {chip} · bar {width}px</span>}</h3>
      <div className="spec-col-note">{note}</div>
      <div ref={ref} className="spec-frame" style={width ? { width: `${width}px` } : undefined}>{children}</div>
    </div>
  );
}

function Body({ status, noteToo }: { status: ManagedCloudStatus; noteToo: boolean }) {
  return (
    <main className="sync-main">
      {noteToo && <WorkspaceNote status={status} />}
      <p className="spec-hint" style={{ marginTop: noteToo && status.state !== "ready" ? 10 : 0 }}>Git history · 14 sessions</p>
    </main>
  );
}

function MarkCard({ id, markStyle }: { id: Mark; markStyle: "composite" | "swap" }) {
  const [ink, setInk] = useState<number | null>(null);
  useEffect(() => setInk(inkOf(id)), [id]);
  const tone: Tone = id === "check" ? "success" : id === "clock" ? "info" : id === "x" ? "danger" : id === "alert" || id === "user" || id === "off" ? "warning" : "plain";
  const used = CASES.filter((item) => headline(item.status).mark === id).map((item) => headline(item.status).word);
  return (
    <div className="spec-mark">
      <div className="spec-mark-sizes">
        <MarkGlyph mark={id} tone={tone} size={13} style={markStyle} />
        <MarkGlyph mark={id} tone={tone} size={16} style={markStyle} />
        <MarkGlyph mark={id} tone={tone} size={24} style={markStyle} />
        <MarkGlyph mark={id} tone={tone} size={72} style={markStyle} />
      </div>
      {markStyle === "composite" && (
        <div className="spec-mark-raster">
          <Raster mark={id} size={16} dpr={1} />
          <Raster mark={id} size={16} dpr={2} />
        </div>
      )}
      <b>{id}</b>
      <small>{[...new Set(used)].join(" · ")}</small>
      {markStyle === "composite" && ink !== null && <small className={ink >= 15 && ink <= 48 ? "" : "spec-bad"}>ink {ink} / 144</small>}
    </div>
  );
}

const MARKS: Mark[] = ["cloud", "dot", "check", "lock", "clock", "x", "alert", "user", "off"];

function Specimen() {
  const [knobs, setKnobs] = useState<Knobs>(readKnobs);
  const set = <K extends keyof Knobs>(key: K, value: Knobs[K]) => {
    const next = { ...knobs, [key]: value };
    setKnobs(next);
    const query = new URLSearchParams({ ...next });
    window.history.replaceState(null, "", `${window.location.pathname}?${query}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const seg = <K extends keyof Knobs>(key: K, value: Knobs[K], label: string) => (
    <button key={String(value)} type="button" className="spec-seg" aria-pressed={knobs[key] === value} onClick={() => set(key, value)}>{label}</button>
  );
  const shows = (dir: Exclude<Dir, "all">) => knobs.dir === "all" || knobs.dir === dir;
  const current = CASES.find((item) => item.id === knobs.state) ?? CASES[3];
  const status = current.status;
  const notReady = status.state !== "ready";
  const hideChip = notReady && knobs.nonready === "notice";
  const chip = (face: Face, pop: ReactNode) => hideChip ? null : (
    <CloudButton status={status} face={face} markStyle={knobs.mark} open>{pop}</CloudButton>
  );

  return (
    <div className="spec">
      <h1 className="spec-title">Mallard Cloud status · choices</h1>
      <p className="spec-sub">
        One Cloud mark in the path bar for every state, and the Cloud verbs in a popover of their own. Menus and notices are the shipped
        components; words marked <i>today</i> come from the shipped functions.
      </p>

      <div className="spec-controls">
        <div className="spec-group"><span>Direction</span>{seg("dir", "all", "All")}{seg("dir", "a", "A")}{seg("dir", "b", "B")}{seg("dir", "c", "C")}</div>
        <div className="spec-group">
          <span>State</span>
          <select className="spec-select" value={knobs.state} onChange={(event) => set("state", event.target.value)}>
            {CASES.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
          </select>
        </div>
        <div className="spec-group"><span>Bar</span>{seg("width", "580", "580")}{seg("width", "800", "800")}{seg("width", "1100", "1100")}</div>
        <div className="spec-group"><span>Mark</span>{seg("mark", "composite", "Cloud + badge")}{seg("mark", "swap", "Swap glyph")}</div>
        <div className="spec-group"><span>Copy</span>{seg("copy", "split", "Split row")}{seg("copy", "links", "Link list")}{seg("copy", "rows", "Copy rows")}</div>
        <div className="spec-group"><span>Not ready</span>{seg("nonready", "mark", "Mark + notice")}{seg("nonready", "notice", "Notice only")}</div>
        <div className="spec-group"><span>Theme</span>{seg("theme", "light", "Light")}{seg("theme", "dark", "Dark")}</div>
      </div>

      <h2 className="spec-h2">1 · Today</h2>
      <div className="spec-row">
        <Frame
          title="The project menu"
          width="760"
          note={<ol className="spec-issues">
            <li>The caption joins two verbs' details into one run-on line.</li>
            <li>View draft and View published version are the same icon pill.</li>
          </ol>}
        >
          <Bar menu={projectMenu(true)} chip={<TodayChip status={status} />} />
          <div className="spec-menu-room" />
        </Frame>
        <Frame
          title="The chip"
          width={knobs.width}
          measure
          note={<ol className="spec-issues" start={3}>
            <li>A label you can't open; the words are in a tooltip.</li>
            <li>"Published" names no version; "Not requested" covers private and a newer draft alike.</li>
            <li>Nothing when the repository is gone or you're signed out: only the notice speaks.</li>
          </ol>}
        >
          <Bar chip={<TodayChip status={status} />} />
          <Body status={status} noteToo />
        </Frame>
      </div>

      <h2 className="spec-h2">2 · The project menu, after (every direction)</h2>
      <p className="spec-lede">The Cloud verbs leave the project menu. What's left is the project's own verbs, and the caption is one fact.</p>
      <div className="spec-row">
        <Frame title="Project menu" width="760" note="Four pills: Reveal, Agent profile, Rename, Delete sessions. The caption is the agent profile alone.">
          <Bar menu={projectMenu(false)} chip={hideChip ? null : <CloudButton status={status} face="mark" markStyle={knobs.mark} open={false} />} />
          <div className="spec-menu-room" />
        </Frame>
      </div>

      <h2 className="spec-h2">3 · The Cloud mark and its popover</h2>
      <p className="spec-lede">
        State: <b>{current.name}</b>. Change it above; every direction redraws. At {knobs.width}px the bar's container rules apply as shipped.
      </p>
      <div className="spec-row">
        {shows("a") && (
          <Frame
            title="A · Mark only"
            pick
            width={knobs.width}
            measure
            note="Like a sync client's menu-bar icon: one cloud, a badge per state, the same at every width. The word is the button's name, its tooltip and the popover's first line."
          >
            <Bar chip={chip("mark", <ListPopover status={status} markStyle={knobs.mark} copy={knobs.copy} />)} />
            <Body status={status} noteToo />
            <div className="spec-pop-room" />
          </Frame>
        )}
        {shows("b") && (
          <Frame
            title="B · Mark + word"
            width={knobs.width}
            measure
            note="A state pill, icon and word, like a pull request's state label. The word shows above 960px; below, the chip folds to A's mark. Same popover."
          >
            <Bar chip={chip("word", <ListPopover status={status} markStyle={knobs.mark} copy={knobs.copy} />)} />
            <Body status={status} noteToo />
            <div className="spec-pop-room" />
          </Frame>
        )}
        {shows("c") && (
          <Frame
            title="C · Mark + word, release track"
            width={knobs.width}
            measure
            note="B's chip; the popover draws the release as three stops, Draft → Review → Public, with the one next step as its button. No store draws a stepper: this is Play Console's three sections (not sent, in review, ready) drawn as a line. Fits under A's mark too."
          >
            <Bar chip={chip("word", <TrackPopover status={status} markStyle={knobs.mark} copy={knobs.copy} />)} />
            <Body status={status} noteToo />
            <div className="spec-pop-room" />
          </Frame>
        )}
      </div>

      <h2 className="spec-h2">4 · Copying the links</h2>
      <p className="spec-lede">
        Every design copies the same three links: the draft, the published version, and the release (the website's Release tab, where the Owner or
        an Admin decides). One row carries the release link in every state: Request public release while it can be sent, View release request while
        it waits or after a rejection. Shown under A's popover; B and C take the same rows. Each button really copies.
      </p>
      <div className="spec-pop-alone">
        {(["split", "links", "rows"] as const).map((style) => (
          <div key={style}>
            <h3 className="spec-col-title">
              {style === "split" ? "Split row" : style === "links" ? "Link list" : "Copy rows"}
              {style === "split" && <em>recommended</em>}
            </h3>
            <div className="spec-col-note">
              {style === "split"
                ? "The row opens; the button at its end copies, and says Copied for two seconds. Two buttons per row, so the verbs are a group the Tab key walks, not a menu."
                : style === "links"
                  ? "The addresses printed under the verbs, as File info prints a path; clicking one copies it. The most explicit and the tallest, and the only one that shows the address."
                  : "Copying as a menu row of its own under each link, as a browser's Copy Link. Keeps a plain menu; doubles the rows."}
            </div>
            {status.link
              ? <ListPopover status={status} markStyle={knobs.mark} copy={style} />
              : <p className="spec-hint">Not ready: there is no repository link to copy. Pick a ready state above.</p>}
          </div>
        ))}
      </div>

      <h2 className="spec-h2">5 · Every state</h2>
      <p className="spec-lede">
        One mark shows one state, so the most urgent wins: sign-in and reach, then a missing repository, then the release (rejected, in review), then
        the draft (newer, published, private), then nothing pushed. Click a row to show it above.
      </p>
      <div className="spec-panel">
        <table className="spec-states">
          <thead>
            <tr><th>#</th><th>Service answer</th><th>Today</th><th>A</th><th>B</th><th>Popover line</th><th>Popover verbs</th><th>Copies</th></tr>
          </thead>
          <tbody>
            {[...CASES].sort((one, two) => headline(one.status).rank - headline(two.status).rank).map((item) => {
              const head = headline(item.status);
              const note = cloudStatusNotice(item.status);
              const verbs = cloudRows(item.status).map((row) => row.label.replace(/…$/, ""));
              return (
                <tr key={item.id} aria-selected={item.id === knobs.state} onClick={() => set("state", item.id)}>
                  <td>{head.rank}</td>
                  <td><b>{item.name}</b><small>{describe(item.status)}</small></td>
                  <td>{item.status.state === "ready" && item.status.link
                    ? <span className="spec-today-chip"><TodayChip status={item.status} /></span>
                    : <small>no chip · notice “{note?.title}”</small>}</td>
                  <td><span className={`sync-path-attention sync-path-cloud-status spec-cloud-button mark${head.tone === "warning" || head.tone === "danger" ? ` ${head.tone}` : ""}`}><MarkGlyph mark={head.mark} tone={head.tone} size={16} style={knobs.mark} /></span></td>
                  <td><span className={`sync-path-attention sync-path-cloud-status spec-cloud-button word${head.tone === "warning" || head.tone === "danger" ? ` ${head.tone}` : ""}`}><MarkGlyph mark={head.mark} tone={head.tone} size={16} style={knobs.mark} /><b>{head.word}</b></span></td>
                  <td><small>{head.line}</small></td>
                  <td><small>{verbs.join(" · ") || (note?.action ? (note.action === "retry" ? "Retry" : "Storage settings") : "—")}</small></td>
                  <td><small>{cloudRows(item.status).flatMap((row) => row.href ? [row.href.short] : []).join(" · ") || "—"}</small></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <h2 className="spec-h2">6 · The marks</h2>
      <p className="spec-lede">
        {knobs.mark === "composite"
          ? "The shipped cloud with a badge bitten out of its corner, on the icon grid. Sizes 13 · 16 · 24 · 72, then what 16px paints on a 1× and a 2× screen. Ink is counted as the icon-set note counts it (band 15–48)."
          : "Swap glyph: the chip shows an existing icon per state instead. No new drawing, but the Cloud identity goes, and the set has no glyph for a newer draft (a commit mark stands in)."}
      </p>
      <div className="spec-marks">{MARKS.map((id) => <MarkCard key={id} id={id} markStyle={knobs.mark} />)}</div>

      <h2 className="spec-h2">7 · Patterns consulted</h2>
      <div className="spec-panel spec-patterns" id="patterns">
        <h4>Badge on a base glyph: sync clients → A</h4>
        <p>
          Dropbox on macOS draws file states as clouds: with a down arrow (online only), with ! (can't sync), with a backslash (ignored), dotted
          (waiting). OneDrive and Dropbox on Windows set a check, arrows or ✕ beside each file. Account trouble (signed out, offline, paused)
          goes on the app's one menu-bar or tray icon. A's mark is that icon.
        </p>
        <h4>State pill, icon and word → B</h4>
        <p>
          GitHub's StateLabel (Draft, Open, Merged, Closed) always pairs icon and word, and Primer says the word must carry the meaning. Vercel's
          status dot takes a label when it stands alone, and changes only when the state does, not on every poll. App Store Connect: one status
          word beside a coloured dot.
        </p>
        <h4>Publish button that changes with state → the popover</h4>
        <p>
          Figma Sites and Framer: Publish becomes Update after the first publish. The panel holds the status, the address, when it was last
          published and what changed. No product puts a dot on the button. Here the facts and the one primary verb, Request public release,
          sit in the popover.
        </p>
        <h4>Review lifecycle → C</h4>
        <p>
          No store draws a stepper. App Store Connect shows one word (Waiting for Review, In Review, Rejected); Play Console groups changes as
          not yet sent for review, in review, and ready to publish; Chrome Web Store: Pending, Published, Rejected. C's track is ours.
        </p>
        <h4>Copying a link → the Copy knob</h4>
        <p>
          GitHub's Code button prints the clone address beside a copy button; Figma's Share dialog has Copy link; browsers put Copy Link in a
          menu of its own. In this app, File info's rows already copy a value on click and say Copied for two seconds.
        </p>
        <h4>Accessibility → every direction</h4>
        <p>
          WCAG 2.2 SC 1.4.1: colour is never the only cue, so every badge has its own shape. SC 1.4.11: 3:1 for the parts of a graphic you need.
          Fluent: an icon-only badge needs a name, and badges at 32px and below are hard to see, so pair them with text or a tooltip. A's button
          is named "Mallard Cloud: Draft v5. …".
        </p>
        <p className="spec-hint">
          Sources: <a href="https://help.dropbox.com/sync/macos-sync-icons">Dropbox macOS icons</a> · <a href="https://support.microsoft.com/en-us/office/what-do-the-onedrive-icons-mean-11143026-8000-44f8-aaa9-67c985aa49b3">OneDrive icons</a> · <a href="https://primer.style/product/components/state-label/">Primer StateLabel</a> · <a href="https://vercel.com/geist/status-dot">Geist status dot</a> · <a href="https://help.figma.com/hc/en-us/articles/31242845959703-Publish-update-or-unpublish-a-site">Figma Sites publish</a> · <a href="https://www.framer.com/help/articles/publishing-your-framer-website/">Framer publishing</a> · <a href="https://developer.apple.com/help/app-store-connect/reference/app-and-submission-statuses">App Store Connect statuses</a> · <a href="https://support.google.com/googleplay/android-developer/answer/9859654?hl=en">Play publishing overview</a> · <a href="https://developer.chrome.com/docs/webstore/review-process">Chrome review</a> · <a href="https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html">WCAG 1.4.1</a> · <a href="https://fluent2.microsoft.design/components/web/react/core/badge/usage">Fluent badge</a>
        </p>
      </div>

      <h2 className="spec-h2">8 · What it changes</h2>
      <div className="spec-panel spec-patterns">
        <p><b>Words.</b> One pure <code>cloudHeadline(status)</code> in managedCloudLink.ts: the precedence above and the words, read by the chip, its name and the popover.</p>
        <p><b>The bar.</b> The Cloud chip becomes a button. The popover isn't a <code>PathBarMenu</code>: a header makes that a switcher, whose verbs turn into icon pills. It needs a status block beside the <code>role="menu"</code> list, where the menu's Note sits today.</p>
        <p><b>Links.</b> <code>cloudReleaseUrl(link)</code> beside <code>cloudVersionUrl</code> (<code>?tab=release</code>); copying goes through the shipped <code>writeClipboardText</code>. A split row holds two buttons, so the popover's verbs are a group, not a <code>role="menu"</code>.</p>
        <p><b>The project menu.</b> Loses View draft, View published version, Request public release and Discard draft.</p>
        <p><b>Marks.</b> Eight cloud composites in Icons.tsx, 31–47 cells of ink. The badges are solid, where the set is hollow; at 16px on a 1× screen they blur (see 6).</p>
        <p><b>Not ready.</b> With no link the popover has no repository name; it shows the storage and the notice's one control.</p>
      </div>
    </div>
  );
}

/* ——— the remock: B, split rows, cloud + badge, every state on the chip ——— */

/** Before the first answer the chip has nothing to say yet. Proposed, not
 *  picked: without it the chip appears only once the status arrives, and
 *  the trail beside it shifts. */
function CheckingChip({ face }: { face: Face }) {
  return (
    <button
      type="button"
      className={`sync-path-attention sync-path-cloud-status spec-cloud-button ${face} spec-checking`}
      aria-label="Mallard Cloud: checking"
      data-tooltip="Checking Mallard Cloud…"
      disabled
    >
      <Icon name="cloud" size={16} />
      {face === "word" && <b>Checking…</b>}
    </button>
  );
}

function FoldRow({ status, width, tip }: { status: ManagedCloudStatus; width: number; tip?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const [read, setRead] = useState<string>("");
  useEffect(() => {
    let live = true;
    void document.fonts.ready.then(() => setTimeout(() => {
      const bar = ref.current?.querySelector<HTMLElement>(".sync-pathbar");
      const chip = ref.current?.querySelector<HTMLElement>(".spec-cloud-button");
      if (!live || !bar || !chip) return;
      const style = getComputedStyle(bar);
      const box = bar.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const word = chip.querySelector("b");
      setRead(`bar ${width}px · content ${Math.round(box)}px · chip ${Math.round(chip.getBoundingClientRect().width)}px · ${word && getComputedStyle(word).display !== "none" ? "word shown" : "mark only"}`);
    }, 60));
    return () => { live = false; };
  });
  return (
    <div className="spec-fold-row">
      <span className="spec-measure">{read}</span>
      <div ref={ref} className="spec-frame" style={{ width: `${width}px` }}>
        <Bar chip={<CloudButton status={status} face="word" markStyle="composite" open={false} tip={tip} />} />
      </div>
    </div>
  );
}

function StateCard({ item }: { item: Case }) {
  const head = headline(item.status);
  return (
    <div className="spec-state-card">
      <div className="spec-state-card-head">
        <span className="spec-rank">{head.rank}</span>
        <div>
          <b>{item.name}</b>
          <small>{describe(item.status)}</small>
        </div>
      </div>
      <div className="spec-state-chips">
        <CloudButton status={item.status} face="word" markStyle="composite" open={false} />
        <CloudButton status={item.status} face="mark" markStyle="composite" open={false} />
        <small>above 960 · below</small>
      </div>
      <div className="spec-pop-alone one">
        <ListPopover status={item.status} markStyle="composite" copy="split" />
      </div>
    </div>
  );
}

function PickedSpecimen() {
  const [knobs, setKnobs] = useState(() => {
    const query = new URLSearchParams(window.location.search);
    const state = query.get("state");
    const theme = query.get("theme") === "dark" ? "dark" as const : "light" as const;
    return { state: state && CASES.some((item) => item.id === state) ? state : "new-draft", theme };
  });
  const set = (next: typeof knobs) => {
    setKnobs(next);
    window.history.replaceState(null, "", `${window.location.pathname}?${new URLSearchParams(next)}`);
    document.documentElement.dataset.theme = next.theme;
  };
  const current = CASES.find((item) => item.id === knobs.state) ?? CASES[3];
  const status = current.status;
  const sorted = [...CASES].sort((one, two) => headline(one.status).rank - headline(two.status).rank);
  const seg = (on: boolean, label: string, onClick: () => void) => (
    <button key={label} type="button" className="spec-seg" aria-pressed={on} onClick={onClick}>{label}</button>
  );
  return (
    <div className="spec">
      <h1 className="spec-title">Mallard Cloud status · the picks</h1>
      <p className="spec-sub">
        B (cloud + badge and the word, folding to the mark at 960px), split rows that copy, and every state on the chip, sign-in and gone
        included, with the notice kept under the bar. Menus, notices and the bar are the shipped pieces.
      </p>

      <div className="spec-controls">
        <div className="spec-group">
          <span>State</span>
          <select className="spec-select" value={knobs.state} onChange={(event) => set({ ...knobs, state: event.target.value })}>
            {sorted.map((item) => <option key={item.id} value={item.id}>{headline(item.status).rank} · {item.name}</option>)}
          </select>
        </div>
        <div className="spec-group"><span>Theme</span>{seg(knobs.theme === "light", "Light", () => set({ ...knobs, theme: "light" }))}{seg(knobs.theme === "dark", "Dark", () => set({ ...knobs, theme: "dark" }))}</div>
      </div>

      <h2 className="spec-h2">1 · In the bar</h2>
      <p className="spec-lede">State: <b>{current.name}</b>. The chip open, as a click leaves it. Each copy button really copies.</p>
      <div className="spec-row">
        <Frame title="Open" width="1100" measure note="The word and a chevron above 960px. The panel: the state, the repository, the facts, then the verbs; each link row copies at its end.">
          <Bar chip={<CloudButton status={status} face="word" markStyle="composite" open><ListPopover status={status} markStyle="composite" copy="split" /></CloudButton>} />
          <Body status={status} noteToo />
          <div className="spec-pop-room" />
        </Frame>
      </div>

      <h2 className="spec-h2">2 · The fold</h2>
      <p className="spec-lede">One rule, as shipped: at a bar content width of 960px or less the word and the chevron go, and the mark stays. Measured live. At the 580px floor the mark costs 2px more than today's (25px against 23px), of the 7px the bar has spare there.</p>
      <div className="spec-fold">
        {[1100, 1010, 990, 800, 580].map((width) => <FoldRow key={width} status={status} width={width} />)}
        <div className="spec-fold-row">
          <span className="spec-measure">hover or focus on the mark: the tooltip says the word</span>
          <div className="spec-frame" style={{ width: "800px" }}>
            <Bar chip={<CloudButton status={status} face="word" markStyle="composite" open={false} tip />} />
            <div style={{ height: 44 }} />
          </div>
        </div>
      </div>

      <h2 className="spec-h2">3 · Every state</h2>
      <p className="spec-lede">
        In order of precedence: the chip shows the first that applies. For a status with no repository link (1–3) the panel names the storage
        and offers the notice's one control; the notice stays under the bar as today.
      </p>
      <div className="spec-state-grid">
        {sorted.map((item) => <StateCard key={item.id} item={item} />)}
        <div className="spec-state-card proposed">
          <div className="spec-state-card-head">
            <span className="spec-rank">0</span>
            <div>
              <b>Before the first answer <em>proposed</em></b>
              <small>no status yet · not one of your picks</small>
            </div>
          </div>
          <div className="spec-state-chips">
            <CheckingChip face="word" />
            <CheckingChip face="mark" />
            <small>inert until the status arrives</small>
          </div>
          <p className="spec-hint">Today the chip is absent until then. Showing it inert keeps the bar from shifting when the answer lands. Drop it if unwanted.</p>
        </div>
      </div>

      <h2 className="spec-h2">4 · Copying</h2>
      <p className="spec-lede">The copy button at a row's end, idle and just after a click (two seconds, then back). The tooltip holds the address.</p>
      <div className="spec-pop-alone">
        <div>
          <h3 className="spec-col-title">Idle</h3>
          <ListPopover status={CASES.find((item) => item.id === "pending-owner")!.status} markStyle="composite" copy="split" />
        </div>
        <div>
          <h3 className="spec-col-title">Copied the release link</h3>
          <ListPopover status={CASES.find((item) => item.id === "pending-owner")!.status} markStyle="composite" copy="split" frozenCopied="release link" />
        </div>
        <div>
          <h3 className="spec-col-title">Copied the draft link</h3>
          <ListPopover status={CASES.find((item) => item.id === "new-draft")!.status} markStyle="composite" copy="split" frozenCopied="draft link" />
        </div>
      </div>

      <h2 className="spec-h2">5 · The project menu</h2>
      <p className="spec-lede">The Cloud verbs have moved to the chip; the project keeps its own four, and the caption is the agent profile alone.</p>
      <div className="spec-row">
        <Frame title="Project menu" width="1100" note="">
          <Bar menu={projectMenu(false)} chip={<CloudButton status={status} face="word" markStyle="composite" open={false} />} />
          <div className="spec-menu-room" />
        </Frame>
      </div>
    </div>
  );
}

function describe(status: ManagedCloudStatus): string {
  if (status.state !== "ready" || !status.link) return `state ${status.state}`;
  const link = status.link;
  const pub = status.publication;
  return [link.visibility, pub?.state ?? "no publication", link.release.kind, link.role ?? "contributor"].join(" · ");
}

const root = document.getElementById("root")!;
document.documentElement.dataset.theme = readKnobs().theme;
createRoot(root).render(<StrictMode>{root.dataset.view === "picked" ? <PickedSpecimen /> : <Specimen />}</StrictMode>);

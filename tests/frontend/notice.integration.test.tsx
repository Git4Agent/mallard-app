import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Notice, {
  Acknowledgement,
  FieldMessage,
  NoticeChip,
  NoticeZone,
  forgetResolved,
  highestSeverity,
  noticeIdentity,
  type NoticeItem,
} from "../../src/components/notice/Notice";

/* The notice system exists to hold three published rules that the app broke in
 * 44 places: one box per zone (GOV.UK), the severity named in the text (USWDS),
 * and no live region for content that is already on screen (WCAG 2.2 SC 4.1.3).
 * These tests pin the rules, not the wording. */

const NOTES: NoticeItem[] = [
  { key: "a", severity: "info", message: "Mallard rebuilt its local timeline cache." },
  { key: "b", severity: "warning", message: "At least one message is shown as a truncated preview." },
];

test("every message closes through one Dismiss control, after its action", () => {
  // Every rung closes — not only warnings — and a message's action does not
  // stop it closing. One control, one word: "Dismiss".
  const html = renderToStaticMarkup(
    <NoticeZone
      items={[
        ...NOTES,
        { key: "c", severity: "warning", message: "Pull first.", action: <button>Open Pull review</button> },
        { key: "d", severity: "danger", message: "Storage failed." },
      ]}
      subject="notices"
      timing="standing"
      label="Notices"
    />,
  );
  assert.equal(html.match(/aria-label="Dismiss: /g)?.length, 5, "each of four lines, and the box itself");
  assert.match(html, /aria-label="Dismiss: all 4 notices"/);
  assert.match(html, /Open Pull review<\/button><button type="button" class="sync-notice-dismiss" title="Dismiss" aria-label="Dismiss: Warning\. Pull first\."/);
  assert.doesNotMatch(html, /Close warning|Dismiss error|Dismiss notification/);
  for (const severity of ["danger", "warning", "info", "success"] as const) {
    const single = renderToStaticMarkup(<Notice severity={severity} timing="inline">Pull first.</Notice>);
    assert.equal(single.match(/class="sync-notice-dismiss"/g)?.length, 1, `${severity} closes once`);
  }
  // The spoken name ends each part as a sentence, never with a double stop.
  const titled = renderToStaticMarkup(<Notice severity="warning" timing="inline" title="Scan incomplete.">1 bucket was skipped.</Notice>);
  assert.match(titled, /aria-label="Dismiss: Warning\. Scan incomplete\. 1 bucket was skipped\."/);
});

test("an acknowledgement is the one message that never closes", () => {
  const html = renderToStaticMarkup(
    <Acknowledgement checked={false} onChange={() => undefined} title="Executable content" verbs={["Mark reviewed", "Reviewed"]}>
      — scripts/install.sh
    </Acknowledgement>,
  );
  assert.match(html, /^<label class="notice warning notice-ack">/);
  assert.match(html, /type="checkbox"/);
  assert.match(html, /Warning ·<\/strong> <strong class="notice-lead">Executable content<\/strong> — scripts\/install\.sh/);
  assert.match(html, />Mark reviewed<\/small>/);
  assert.doesNotMatch(html, /sync-notice-dismiss/);
});

test("each rung has its own glyph: an error is the octagon, never the warning triangle", () => {
  const glyph = (severity: "danger" | "warning" | "info" | "success") => {
    const icon = renderToStaticMarkup(
      <Notice severity={severity} timing="inline">Text.</Notice>,
    ).match(/<svg[^>]*class="[^"]*notice-mark[^"]*"[^>]*>/)?.[0];
    return icon?.match(/data-icon="([^"]+)"/)?.[1];
  };
  assert.equal(glyph("danger"), "octagon-x");
  assert.equal(glyph("warning"), "alert-triangle");
  assert.equal(glyph("info"), "info");
  assert.equal(glyph("success"), "check-circle");
  assert.match(renderToStaticMarkup(<FieldMessage>Choose a folder.</FieldMessage>), /data-icon="octagon-x"/);
  assert.match(renderToStaticMarkup(<NoticeChip severity="danger">1</NoticeChip>), /data-icon="octagon-x"/);
});

test("a field message has no box, speaks its type, and closes", () => {
  const html = renderToStaticMarkup(<FieldMessage id="field-error">This folder no longer exists.</FieldMessage>);
  assert.match(html, /^<div id="field-error" class="notice-field">/);
  assert.match(html, /<span class="sync-visually-hidden">Error: <\/span>This folder no longer exists\./);
  assert.match(html, /aria-label="Dismiss: Error\. This folder no longer exists\."/);
  assert.doesNotMatch(html, /role=/);
  assert.match(renderToStaticMarkup(<FieldMessage blocking>Taken.</FieldMessage>), /role="alert"/);
});

test("a closed message is forgotten once it resolves, so it shows again if it recurs", () => {
  const failed: NoticeItem = { key: "push", severity: "danger", message: "Push failed." };
  const closed = new Set([noticeIdentity(failed)]);
  // Still present: it stays closed, and the set is kept as is.
  assert.equal(forgetResolved(closed, [failed]), closed);
  // Gone for one render — the retry cleared it — so it is forgotten, and the
  // same failure on the next attempt is shown again.
  assert.equal(forgetResolved(closed, []).size, 0);
  // A counter in the identity separates two failures with the same words.
  const retried = { ...failed, dismissalId: "attempt-2" };
  assert.notEqual(noticeIdentity(retried), noticeIdentity(failed));
  assert.equal(forgetResolved(closed, [retried]).size, 0);
});

test("a zone is one box, whatever the message count", () => {
  const two = renderToStaticMarkup(
    <NoticeZone items={NOTES} subject="notes about this record" timing="standing" label="Notes" />,
  );
  assert.equal(two.match(/class="notice /g)?.length, 1, "one box, not two");
  assert.equal(two.match(/<li>/g)?.length, 2, "both messages present");

  const four = renderToStaticMarkup(
    <NoticeZone
      items={[...NOTES, { key: "c", severity: "info", message: "Third." }, { key: "d", severity: "info", message: "Fourth." }]}
      subject="notes about this record"
      timing="standing"
      label="Notes"
    />,
  );
  assert.equal(four.match(/class="notice /g)?.length, 1, "still one box at four messages");

  // A single message does not get a count line it does not need.
  const one = renderToStaticMarkup(
    <NoticeZone items={[NOTES[0]]} subject="notes about this record" timing="standing" label="Notes" />,
  );
  assert.doesNotMatch(one, /<ul|1 notes/);
});

test("the box takes the loudest rung present, and says which it is", () => {
  assert.equal(highestSeverity(["info", "warning", "success"]), "warning");
  assert.equal(highestSeverity(["info", "danger", "warning"]), "danger");
  assert.equal(highestSeverity([]), null);

  const mixed = renderToStaticMarkup(
    <NoticeZone items={NOTES} subject="notes about this record" timing="standing" label="Notes" />,
  );
  assert.match(mixed, /class="notice warning"/);
  assert.match(mixed, /Warning ·/);

  // An info-only zone must not borrow the warning tone. USWDS reserves warning
  // for information that may require action; a note about how the index was
  // built asks nothing of the reader.
  const quiet = renderToStaticMarkup(
    <NoticeZone items={[NOTES[0]]} subject="notes about this record" timing="standing" label="Notes" />,
  );
  assert.match(quiet, /class="notice info"/);
  assert.match(quiet, /Note ·/);
  assert.doesNotMatch(quiet, /Warning/);

  // A mixed zone wears one tint, so the ink alone would say "warning" over a
  // note about how the cache was built. Each message names its own rung.
  assert.match(mixed, /Note ·/);
  assert.equal(mixed.match(/Warning ·/g)?.length, 2, "the title and the one warning message");

  // A zone where every message agrees needs no such repetition: the title
  // already said it once.
  const uniform = renderToStaticMarkup(
    <NoticeZone
      items={[NOTES[0], { key: "c", severity: "info", message: "Second note." }]}
      subject="notes about this record"
      timing="standing"
      label="Notes"
    />,
  );
  assert.equal(uniform.match(/Note ·/g)?.length, 1, "the title only");
});

test("severity survives without the tint or the glyph", () => {
  // WCAG 1.4.1: the type name is in the text, so the meaning does not rest on
  // the amber or on the triangle. Strip every class and every svg and the
  // sentence still says what it is.
  for (const [severity, word] of [["danger", "Error"], ["warning", "Warning"], ["info", "Note"], ["success", "Done"]] as const) {
    const html = renderToStaticMarkup(
      <Notice severity={severity} timing="standing" label="Zone">Something happened.</Notice>,
    );
    const text = html.replace(/<svg[\s\S]*?<\/svg>/g, "").replace(/<[^>]+>/g, "");
    assert.match(text, new RegExp(`^${word} ·`), `${severity} names itself`);
  }
});

test("the role is derived from timing, so standing content never announces", () => {
  const standing = renderToStaticMarkup(
    <Notice severity="warning" timing="standing" label="Notes about this session">Already here.</Notice>,
  );
  assert.match(standing, /role="region" aria-label="Notes about this session"/);
  assert.doesNotMatch(standing, /aria-live|role="status"|role="alert"/);

  const progress = renderToStaticMarkup(
    <Notice severity="info" timing="progress">Pulling…</Notice>,
  );
  assert.match(progress, /role="status"/);

  const blocking = renderToStaticMarkup(
    <Notice severity="danger" timing="blocking">The push failed.</Notice>,
  );
  assert.match(blocking, /role="alert"/);

  // role="status" already implies aria-live="polite"; writing both is noise.
  assert.doesNotMatch(progress, /aria-live/);
});

test("the stylesheet has one recipe per tier, and the eleven one-off treatments are gone", () => {
  const css = readFileSync("src/App.css", "utf8");
  // A strip is the notice flush in a card: the rung's edge on top only.
  assert.match(css, /\.notice\.notice-strip \{[^}]*border-width: 1px 0 0;[^}]*border-radius: 0;/);
  // Every chip rung is the notice rung's ink on its tint.
  for (const rung of ["danger", "warning", "info", "success"]) {
    assert.match(css, new RegExp(`\\.notice-chip\\.${rung} \\{ color: var\\(--${rung}-text\\); background: var\\(--${rung}-soft\\); \\}`));
  }
  // A finished sync's strip takes the notice tokens, not a private mix.
  assert.match(css, /\.sync-outcome-notice\.success \{ border-color: color-mix\(in srgb, var\(--green\) 45%, var\(--border\)\); background: var\(--success-soft\); \}/);
  // The shell banner's text is the notice's 12px, not a size of its own.
  assert.doesNotMatch(css.match(/\.sync-backend-banner \{[^}]*\}/)?.[0] ?? "", /font-size/);
  // The one-off treatments this replaced, and the wrapper rules that
  // silently restored a grey, untinted "warning", stay deleted.
  assert.doesNotMatch(
    css,
    /\.notice\.sync-project-file-conflict|\.notice\.sync-storage-notice|\.sync-path-menu-banner|\.sync-setup-field-error|\.sync-setup-acknowledge|\.catalog-card-ack|\.sync-storage-index-warning|\.sync-project-file-warning/,
  );
});

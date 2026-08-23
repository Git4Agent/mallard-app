import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import Notice, {
  NoticeZone,
  highestSeverity,
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

test("the session-detail page says everything it has to say in one box", () => {
  const page = readFileSync(
    "src/components/project-sync/session-detail/SessionDetailPage.tsx",
    "utf8",
  );
  // The action failure, the moved snapshot and the backend's notes were three
  // separate amber rows across two components. They are three rungs of one
  // ladder now, and the page renders the zone exactly once.
  assert.equal(page.match(/<NoticeZone/g)?.length, 1);
  assert.doesNotMatch(page, /sync-callout|session-detail-warnings|session-detail-stale/);
  assert.doesNotMatch(page, /role="status"|role="alert"/);

  const header = readFileSync(
    "src/components/project-sync/session-detail/SessionDetailHeader.tsx",
    "utf8",
  );
  assert.doesNotMatch(header, /actionError|sync-callout/);
});

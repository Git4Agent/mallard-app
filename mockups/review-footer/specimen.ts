// @ts-nocheck
/*
 * Side-by-side page for the review footer specimen: every frame is the real
 * app (live.html) cropped to its review, loaded one at a time.
 *
 *   /mockups/review-footer/index.html?theme=light|dark&lang=zh|en&only=A,B
 */
const params = new URLSearchParams(window.location.search);
const theme = params.get("theme") === "dark" ? "dark" : "light";
const lang = params.get("lang") === "en" ? "en" : "zh";
const only = params.get("only")?.split(",") ?? null;
document.documentElement.dataset.theme = theme;

type Frame = { view: "push" | "pull"; step?: "files"; fix: boolean; dir?: string; caption: string; measure?: boolean };
type Row = { key: string; title: string; seen?: string; against?: string; pick?: boolean; frames: Frame[] };

const link = (name: string, url: string) => `<a href="${url}" target="_blank" rel="noreferrer">${name}</a>`;

const pair = (dir: string): Frame[] => [
  { view: "push", fix: true, dir, caption: "Push" },
  { view: "pull", fix: true, dir, caption: "Pull" },
];

const sections: { title: string; rows: Row[] }[] = [
  {
    title: "1 · Bug: the last row's shadow runs under the footer",
    rows: [
      {
        key: "before",
        title: "Before",
        frames: [
          { view: "push", fix: false, caption: "Push · Sessions", measure: true },
          { view: "pull", fix: false, caption: "Pull · Sessions", measure: true },
          { view: "push", step: "files", fix: false, caption: "Push · Files", measure: true },
          { view: "pull", step: "files", fix: false, caption: "Pull · Files", measure: true },
        ],
      },
      {
        key: "fix",
        title: "Fix — one 12px gutter inside the scroller (project Push/Pull, both steps; same 8px at 953/682/598/444)",
        frames: [
          { view: "push", fix: true, dir: "fix", caption: "Push · Sessions", measure: true },
          { view: "pull", fix: true, dir: "fix", caption: "Pull · Sessions", measure: true },
          { view: "push", step: "files", fix: true, dir: "fix", caption: "Push · Files", measure: true },
          { view: "pull", step: "files", fix: true, dir: "fix", caption: "Pull · Files", measure: true },
        ],
      },
    ],
  },
  {
    title: "2 · Make “Project files · 1 new” visible (all with the fix)",
    rows: [
      {
        key: "A",
        title: "A · Box button",
        pick: true,
        seen: `${link("material-web Button", "https://github.com/material-components/material-web/blob/main/docs/components/button.md")}, ${link("Carbon Button", "https://carbondesignsystem.com/components/button/usage/")}, ${link("HIG Buttons", "https://developer.apple.com/design/human-interface-guidelines/buttons")}`,
        against: "Three boxes beside Push",
        frames: pair("A"),
      },
      {
        key: "B",
        title: "B · Tab badge",
        seen: `${link("HIG Tab bars", "https://developer.apple.com/design/human-interface-guidelines/tab-bars")}, ${link("Compose Badges", "https://developer.android.com/develop/ui/compose/components/badges")}, ${link("Primer UnderlineNav", "https://primer.style/product/components/underline-nav/guidelines/")}`,
        against: `Passive badges missed (${link("NN/g", "https://www.nngroup.com/articles/indicators-validations-notifications/")}); two numbers`,
        frames: pair("B"),
      },
      {
        key: "C",
        title: "C · Inline notice",
        seen: `${link("Primer Banner", "https://primer.style/product/components/banner/guidelines/")}, ${link("Carbon Notification", "https://carbondesignsystem.com/components/notification/usage/")}, ${link("Atlassian Section message", "https://atlassian.design/components/section-message/usage")}`,
        against: "Banners: sparingly, one at once",
        frames: pair("C"),
      },
      {
        key: "D",
        title: "D · Footer note",
        seen: `${link("git status", "https://git-scm.com/book/en/v2/Git-Basics-Recording-Changes-to-the-Repository")}, ${link("Carbon Notification", "https://carbondesignsystem.com/components/notification/usage/")} (above submit), Mallard's over-limit row`,
        against: "Costs the footer a row",
        frames: pair("D"),
      },
      {
        key: "E",
        title: "E · Attention pill",
        seen: `${link("Primer Label", "https://primer.style/product/components/label/")}, ${link("Fluent 2 Badge", "https://fluent2.microsoft.design/components/web/react/core/badge/usage")}, Mallard's blocker pill`,
        against: `Tags aren't links (${link("Carbon", "https://carbondesignsystem.com/components/tag/usage/")})`,
        frames: pair("E"),
      },
      {
        key: "F",
        title: "F · Box button + verb names what goes",
        seen: `${link("GitHub Desktop", "https://github.com/desktop/desktop/blob/development/app/src/ui/changes/commit-message.tsx")} “Commit N files”, ${link("NN/g", "https://www.nngroup.com/articles/confirmation-dialog/")} (name the counts), + A`,
        against: "Repeats the tab counts",
        frames: pair("F"),
      },
      {
        key: "G",
        title: "G · Confirm on Push",
        seen: `${link("VS Code smart commit", "https://github.com/microsoft/vscode/blob/main/extensions/git/src/commands.ts")}, ${link("GitHub Desktop", "https://github.com/desktop/desktop/blob/development/app/src/ui/changes/confirm-commit-filtered-changes-dialog.tsx")}`,
        against: `Reflexive clicks (${link("NN/g", "https://www.nngroup.com/articles/confirmation-dialog/")}); nobody blocks omissions`,
        frames: pair("G"),
      },
    ],
  },
];

/* Toggles reload the page with the new query. */
function toggles(id: string, key: string, options: [string, string][], current: string) {
  const host = document.getElementById(id);
  for (const [value, label] of options) {
    const button = document.createElement("button");
    button.textContent = label;
    button.setAttribute("aria-pressed", String(value === current));
    button.onclick = () => {
      params.set(key, value);
      window.location.search = params.toString();
    };
    host.append(button);
  }
}
toggles("theme", "theme", [["light", "Light"], ["dark", "Dark"]], theme);
toggles("lang", "lang", [["zh", "中文"], ["en", "English"]], lang);

const queue: (() => Promise<void>)[] = [];
const rows = document.getElementById("rows");
let frameId = 0;

for (const section of sections) {
  const visible = section.rows.filter((row) => !only || only.includes(row.key));
  if (visible.length === 0) continue;
  const heading = document.createElement("h2");
  heading.textContent = section.title;
  rows.append(heading);
  for (const row of visible) {
    const block = document.createElement("section");
    block.className = "row";
    block.dataset.row = row.key;
    block.innerHTML = `<h3>${row.title}${row.pick ? '<span class="pick">Recommended</span>' : ""}</h3>`
      + (row.seen ? `<p>Seen in: ${row.seen}${row.against ? ` · <span class="against">Against: ${row.against}</span>` : ""}</p>` : "");
    const frames = document.createElement("div");
    frames.className = "frames";
    block.append(frames);
    rows.append(block);
    for (const frame of row.frames) {
      const id = `f${frameId++}`;
      const figure = document.createElement("figure");
      figure.innerHTML = `<figcaption><b>${frame.caption}</b><span class="m"></span></figcaption><div class="crop loading"></div>`;
      frames.append(figure);
      const crop = figure.querySelector(".crop") as HTMLElement;
      const query = new URLSearchParams({ view: frame.view, theme, lang, id });
      if (frame.step) query.set("step", frame.step);
      if (frame.fix) query.set("fix", "1");
      if (frame.dir) query.set("dir", frame.dir);
      let attempts = 0;
      queue.push(() => new Promise((resolve) => {
        const iframe = document.createElement("iframe");
        iframe.src = `./live.html?${query}`;
        iframe.title = `${row.title} — ${frame.caption}`;
        const done = (event: MessageEvent) => {
          if (event.data?.kind !== "review-footer-frame" || event.data.id !== id) return;
          window.removeEventListener("message", done);
          clearTimeout(timer);
          // A cold first load can miss its waits; one fresh try.
          if (event.data.error && attempts++ === 0) {
            console.warn(`[specimen] ${id}: ${event.data.error}; retrying`);
            window.addEventListener("message", done);
            arm();
            iframe.src = `./live.html?${query}&retry=1`;
            return;
          }
          const measure = event.data.measure;
          if (measure) {
            const pad = 8;
            iframe.style.left = `${-(measure.rect.x - 2)}px`;
            iframe.style.top = `${-(measure.rect.y - 2)}px`;
            crop.style.width = `${Math.ceil(measure.rect.width + pad)}px`;
            crop.style.height = `${Math.ceil(measure.rect.height + pad)}px`;
            if (frame.measure && measure.clear !== null) {
              const label = figure.querySelector(".m");
              label.className = measure.clear < 0 ? "m bad" : "m ok";
              label.textContent = `shadow → footer ${measure.clear}px`;
            }
          }
          crop.classList.remove("loading");
          resolve();
        };
        // A frame that never reports gets one fresh load, then is left as is.
        let timer;
        const arm = () => {
          timer = setTimeout(() => {
            if (attempts++ === 0) {
              arm();
              iframe.src = `./live.html?${query}&retry=1`;
              return;
            }
            window.removeEventListener("message", done);
            resolve();
          }, 45_000);
        };
        arm();
        window.addEventListener("message", done);
        crop.append(iframe);
      }));
    }
  }
}

(async () => {
  for (const load of queue) await load();
  document.documentElement.dataset.shotReady = "1";
})();

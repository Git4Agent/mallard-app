# Simplified Chinese for Mallard — desktop and website

Status: **desktop and website implemented October 8, 2026 (website not
deployed).** Website: W1–W4 done in
`mallardInternal/cloudflare/managed-cloud/web` — 9 namespaces (shell, common,
format, hub, repository, content, session, landing, docs incl. ≈2.9k words of
docs), zh-CN offered; the bar's Auto · English · 简体中文 switch, a Language row
in ⋯, one button naming the other language on phones. Checks: 413 web tests
(also under `LANG=zh_CN`), 233 Worker tests, `tsc`, `build:web`, the guard at 0
over every translated source; 86 harness screens (`web/i18n-live.html`,
`output/i18n-impl/harvest-site.mjs`) give English identical to HEAD except the
switch and two plural fixes ("1 prompt · 1 reply · 1 tool call", "1 agent
spawned"), and Chinese leaves only user data, transcripts, product names,
service sentences, log lines, Super Admin and the Privacy body in English.
Website-only terms: Reviews (release approval) = 审核; Provider = 提供方; an
item = 资源 (as the desktop). Owner: "Looks good, implement the desktop first" — the
recommended option of every open decision below was taken. Desktop D1–D8 are
in the tree: i18next 26.4.2 + react-i18next 17.0.15 with plain string keys
(measured: tsc 13.5 s with a 2.8k-key typed catalog vs 15.2 s for selectors);
15 namespaces in `src/i18n/{en,zh-CN}`, zh typed against en; a value-spacing
formatter that settles Han/Latin spacing around interpolated values; backend
fixed labels translated by key through `backendText` (`src/i18n/backend.ts`)
with new Rust codes where none existed (discovery facts, count explanations,
sync details, readiness, progress, after-Pull offers); the native menu from
`src-tauri/src/app_menu.rs` via `set_ui_language`; `CFBundleLocalizations`;
Settings › Appearance › Language and a quick switch in the app menu.
Checks: 682 frontend tests, every workspace crate's lib tests, both tsc
configs, `npm run build`, `scripts/i18n-guard.mjs` at 0 over all of `src/`,
English text of 76 harness screens identical to the pre-change baseline,
and a Chinese harvest of the same screens leaving only user data, backend
errors and product names in English. Open: the live Tauri pass on macOS and
the Windows WebView2 language check. The specimen board now draws from the
real catalogs (`?copy=app`, the default). Specimens:
`mockups/i18n-zh-mockup.html` (desktop, round 2: the shipped shell live on a
stand-in backend, `?lang=zh|en&font=cjk|default&theme=light|dark`, frames
`mockups/i18n-zh-live.html?screen=…`) and, in mallardInternal,
`cloudflare/managed-cloud/web/i18n-zh-mockup.html` (website). Both draw the
real components; a specimen-only shim rewrites rendered English, so layouts
are true and the copy is a draft. Captures:
`output/i18n-zh-mock-2026-10-07/` in each repo (r2–r7 for the later rounds).

Desktop coverage (rounds 2–7): Settings with the Language row; sessions,
Push and Pull reviews (project and one session), the Files tab and Push with
files; every storage setup step (Mallard Cloud, R2 create and use-existing,
Dropbox, local folder) and the storage pages; project add and setup, the
project menu, rename, agent profile, Link storage; the Mallard Cloud panel in
five states and Publish version; a session's page with the whole timeline
(a Chinese and an English transcript, every event kind, an opened tool call,
an agent row and the agent's own page); the Library (empty, a Skills row in
every state, item pages, Add resource) with its Push and Pull reviews, their
outcomes, Overwrite approval, fork and a refused fork, on R2 and on Mallard
Cloud; Open in Mallard sheets; Logs; the update pill; the startup screen;
native confirms (drawn from the payload Mallard sends macOS), including a
delete Codex refuses (session in use, close Codex first, Codex closed, and
Delete all stopped part-way). Any other live
harness renders in Chinese through `mockups/i18n-zh-wrap.html?h=<harness>`.
Not mocked: Feishu (the type chooser filters it out, `StorageSettings.tsx:675`).

## Problem

Every word Mallard shows is English, hard-coded in components, model helpers
and Rust. There is no catalog, no language setting, no Chinese font stack,
and dates, counts and sizes are formatted with English rules. About 2,800
user-visible strings live in the desktop frontend and 1,650 in the website.

## Decided (owner, 2026-10-07)

- **One added language: Simplified Chinese (zh-CN).** English stays the
  default and the source.
- **Terms:** Push = 上传, Pull = 下载. *Version* stays English (Version 2,
  发布 Version, 最新 Version). *Skill* stays English too, singular as
  Chinese has no plural, spaced from the Han around it (Skill · Claude,
  9 个 Skill, 搜索 Skill…, 复刻 Skill; on the website 项目、Skill 和桌面宠物). Credential redaction = 密钥隐藏; its hint
  names the scope (密钥、令牌和密码 — the rules cover API keys, tokens,
  private keys, JWTs and passwords). Landing headline: 为 [Codex/Claude
  marks] 打造的Git.
- **Desktop switch: A.** A *Language* row under *Theme* in Settings ›
  Appearance: 跟随系统 · English · 简体中文 (System · English · 简体中文 in
  English). Language names are written in their own language and carry a
  `lang` attribute. The choice applies at once, no restart.
- **Backend errors stay English.** Rust error strings (`Result<_, String>`,
  ~114 commands), activity-log lines, the service's `detail`, and the
  Worker's OAuth failure page. The frontend translates only its own words
  around them ("上传失败：" + the English sentence).
- **What the user typed is never translated:** project, storage and
  repository names, repository and GitHub descriptions, session titles,
  commit subjects, branch names, paths, transcript content, skill, plugin
  and pet names and descriptions. Sample sessions in pictures and demos
  count as typed (owner, 2026-10-08): the landing demo's and the docs'
  session titles, prompts and agent replies stay in English as constants
  (`landing/demoStory.ts`, `docs/pages.tsx`), never catalog keys.

### Glossary

Owner, October 8: "Go with your recommendations" — every proposed term below is
decided as written.

| English | 中文 | |
|---|---|---|
| Push / Pull | 上传 / 下载 | decided |
| Version | Version | decided: stays English |
| Skill / Skills | Skill | decided: stays English, never 技能 |
| Review (Push/Pull review, mark reviewed) | 检查 | decided 2026-10-08: 上传检查, 下载检查, 标为已检查 — never 审阅 |
| Claimed by another storage | 同时关联了 … | decided 2026-10-08: never 固定 |
| Credential redaction | 密钥隐藏 | decided |
| Mallard, Mallard Cloud, Codex, Claude, GitHub | unchanged | product names |
| Session | 会话 | decided 2026-10-08 |
| Repository | 仓库 | decided 2026-10-08 (GitHub Docs and JetBrains UI use 存储库) |
| Storage | 存储 | decided 2026-10-08 |
| Library (Catalog) | 资源库 | decided 2026-10-08 |
| Subagent | 子代理 | decided 2026-10-08 |
| Snapshot | 快照 | decided 2026-10-08 |
| Draft | 草稿 | decided 2026-10-08 |
| Release (make public) | 公开发布 | decided 2026-10-08 |
| Project | 项目 | decided 2026-10-08 |
| Prompt / Reply / Step | 提示词 / 回复 / 步骤 | decided 2026-10-08 (session page filters) |
| Checkpoint (a commit made in a session) | 检查点 | decided 2026-10-08 |
| Agent (a delegated run) | 代理 | decided 2026-10-08; the row's word, beside Subagent 子代理 |
| Brief / Report / Transcript | 任务说明 / 报告 / 对话记录 | decided 2026-10-08 |
| async / sync / spawn | 异步 / 同步 / 派生 | decided 2026-10-08 |
| Fork | 复刻 | decided 2026-10-08 (GitHub's Chinese UI word) |
| Bash, Read, Edit, web_search; model ids; effort (high, medium) | unchanged | the agent's own names, shown as recorded |
| Website Reviews (a release request waiting on approval) | 审核 | decided 2026-10-08; Approve 批准, Decline 拒绝 — a different act from a Push/Pull 检查 |
| Provider (Codex, Claude) | 提供方 | decided 2026-10-08 |
| Item (a Library item) | 资源 (资源库资源) | decided 2026-10-08, as the desktop |
| Download (a file) | 下载 | decided 2026-10-08: the same word as Pull; the object (下载 .zip) tells them apart |
| Versions (the tab) | Version 列表 | decided 2026-10-08 |
| Review a PR (landing use case) | 审查 PR | decided 2026-10-08 |
| Super Admin | Super Admin | decided 2026-10-08: stays English, in prose too |
| Feishu · App version · Requirements | 飞书 · 软件更新 · 依赖 | decided 2026-10-08 (desktop) |
| Public on Mallard (landing section) | Mallard 上的公开仓库 | decided 2026-10-08 (owner wording) |

## Open decisions (recommendation first)

1. **Simplified only — confirm.** Assumed throughout; never stated.
2. **Traditional-Chinese systems** (zh-TW, zh-HK, zh-Hant): follow the
   system as **English** (recommended: Simplified text reads as foreign
   there) or as Simplified. Either way the user can pick 简体中文.
3. **Backend-provided fixed labels — translate by key in the frontend
   (recommended).** These are not errors: the specimen shows them
   translated. The inventory:
   - *Key already beside the text* — storage provider descriptors (~60
     strings: names, descriptions, field and secret labels, placeholders,
     help, setup doors, discovery hints; `storage_providers/*.rs`, keyed by
     `key` + `field_id`/`secret_id`), storage summary labels (~8,
     `field_id`), resource kind and apply-policy labels (5 + 5,
     `kind.key`/`apply_policy.key`), catalog labels (~9, coordinate),
     agent `profile_label` and maintenance actions, session actions
     ("Open in Codex", `action_id`), conversation `blocked_reason` (9,
     `blocker`; the frontend already has its own copy for 6), readiness
     issues (`issue_id`), and session-detail and catalog warning codes.
   - *Key must be added in Rust* — R2/Dropbox discovery facts, scope and
     issues (`r2_setup.rs:351-416`, `dropbox_setup.rs:381-410`, including
     R2's "Created" date formatted with an English month in Rust), catalog
     `scope_label` and holding root summaries, `SessionCount.explanation`
     (~6), catalog progress messages ("Pushing {name}…"), after-pull
     "Restart {}", one `disabled_reason`.
   - The alternative is to leave all of them English, like errors. Cheaper,
     but setup screens would be half English.
4. **Backend-suggested default names stay as they are.** "R2 storage",
   "Mallard Cloud {login}", "R2 {bucket}" are saved as the storage's name
   (`storage_setup.rs:499,756`) — user data once saved, so the user-input
   rule applies. Only the field's *placeholder* is translated.
5. **Website switch: A** — the same segmented control beside Theme in the
   signed-out bar, and a *Language* row beside Theme in the ⋯ menu when
   signed in (USWDS would use a menu for three choices; Theme already sets
   the pattern here).
6. **Website addresses: U1** — same URLs, choice remembered in the browser.
   Chinese pages are not indexed by search engines. A `/zh/` prefix collides
   with `/:owner`; a `cn.` host means a second sign-in.
7. **Website content:** translate the docs (≈2.3k words) in W3;
   **keep Privacy in English** with one Chinese line saying so (legal text);
   **keep Super Admin English** (owner-only tooling). Docs videos record
   the English UI and stay as they are.
8. **Native macOS menu: translate** (File, Edit, View, Window, Help and the
   predefined items are English in tauri 2.11.3 and muda 0.19.3). Rust
   rebuilds the menu when the language changes. Also pass button labels on
   the 9 `confirm()` calls that use OS defaults, and on the two `message()`
   calls in `sessionDeletion.ts` (Session in use, Codex closed) that take
   the default Ok, so dialog buttons follow Mallard's language rather than
   the OS's.

## What the user sees

- **First launch:** Mallard follows the system. A Mac whose first preferred
  language is Chinese gets Chinese; anything else gets English. Verified:
  WKWebView reports `navigator.language = zh-CN` when Chinese is first,
  with or without `CFBundleLocalizations` (probe, 2026-10-07).
- **Settings › Appearance › Language:** picking a value re-renders the whole
  app at once; the row's receipt says 已保存 like Theme's.
- **Everywhere:** chrome, menus, notices, empty states, setup flows and
  reviews in Chinese; user data, product names and backend sentences as
  they are. Dates follow Chinese order (2026年10月7日 21:09), counts use 万
  (18.9万), relative times read 8分钟前. Sizes keep B/KB/MB.
- **Type:** Latin stays Baloo 2; Han falls back to PingFang SC (macOS),
  Microsoft YaHei UI (Windows) or Noto Sans CJK SC (Linux). Under
  `:lang(zh)` the 9–10px steps rise to 11px and letter-spacing is normal.
- **Website:** the bar's switch; the landing, hub pages and docs in
  Chinese; English `detail` sentences inside Chinese notices.

## Design

### Library and catalogs

- **i18next 26.4.2 + react-i18next 17.0.15** in both repos (17.0.15 has
  been out since Sept 21; 17.0.16 is a day old). Peer range covers React 19
  and TypeScript 5 (desktop 5.8) through 7 (website 7.0.2). Chosen over
  react-intl (heavier ICU syntax, no gain for two languages), Lingui (needs
  a macro transform in Vite *and* the esbuild test bundle) and a hand-rolled
  dictionary (would reimplement plurals, interpolation and `<Trans>`).
- **Catalogs are TypeScript modules,** one namespace per feature area,
  following the source map: `src/i18n/en/<ns>.ts` exported `as const`, and
  `src/i18n/zh-CN/<ns>.ts` typed against English, so a missing or extra key
  fails `tsc`. Interpolation values are typed. English plural pairs
  (`_one`/`_other`) map to a single Chinese `_other`; the parity type
  allows that and nothing else.
- **The English catalog is today's copy verbatim.** Rendered output in
  English does not change, so rendered test assertions keep passing.
- **Initialized synchronously** at module load with bundled resources (no
  fetching), so tests and the first render need no setup.
- **Keys:** decided in D1 by measurement — plain string keys if `tsc` stays
  fast with a full-size (~2.8k) typed catalog, i18next's selector API
  (`enableSelector`) if it does not. Switching later rewrites every call.

### Language resolution

- `mallard.language` in `src/preferences.ts` beside `mallard.theme`:
  absent = system, or `en` / `zh-CN`. Applied in `main.tsx` before React
  mounts, with the theme: sets i18next's language and `<html lang>`.
- System = the first entry of `navigator.languages` that is English or
  Chinese; Simplified Chinese (`zh`, `zh-CN`, `zh-Hans*`, `zh-SG`) → zh-CN;
  Traditional per decision 2; else English.
- Website: the same key and rule in `web/src/language.ts` beside `theme.ts`.
  The site's CSP (`script-src 'self'`) refuses inline scripts, so a same-origin
  classic script, `web/public/boot-language.js`, sets `<html lang>` in `<head>`
  before the first paint; the boot markup carries one line per language and
  `html:lang(zh)` picks it. A test runs the script against `resolveLanguage`
  for a matrix of choices and browser lists, and pins its offered list.
- Tests on both sides pin an English locale (desktop runner, website
  `web/vitest.config.ts`): Node reports `navigator.languages` from the system
  locale, and under `LANG=zh_CN.UTF-8` 341 of 682 desktop tests rendered
  Chinese.

### Formatting

- `clock.ts`, `format.ts` and the session-detail helpers take the active
  locale instead of `undefined`. Formatters cached at module level
  (`clock.ts:50-65`, `projectFolder.ts:214`, `ProjectActivityPage.tsx:388`)
  are keyed by locale.
- The two hand-rolled "5m ago" helpers become `Intl.RelativeTimeFormat`;
  compact K/M becomes `Intl.NumberFormat` compact; the five literal
  " and " joins become `Intl.ListFormat`; durations and plural phrases move
  to catalog entries with `count`. `formatFullDateTime`'s " at " goes.
- `localeCompare` keeps today's behaviour (names are user data).

### Copy that drives logic — replace with keys first

- `storage-catalog/model.ts:290-300` `actionLabel()` read back with
  `startsWith("Keep local as")` (`StorageCatalogPage.tsx:148-161,664`).
- `itemKindLabel(...) === "Conversation"` (`pullReviewModel.ts:323,347,356`,
  `RestorePlanView.tsx:248,256,437`).
- `workspaceTitle === "Projects"` (`ProjectSync.tsx:3081`; already misfires
  for a draft named "Projects"), `word === "Push"`
  (`StorageCatalogPage.tsx:1611`), `listingPage() === "Catalog"`
  (`appLink.ts:154`).
- Comparisons against backend English keep working and stay
  (`outcomes.ts:35` "Not pulled", `storageError.ts:49-55`, R2's
  "Jurisdiction" until the facts get keys).

### Sentences built from pieces

- English grammar around backend nouns (`StorageDiscovery.tsx` unit /
  unit_plural, `catalogNoun` lowercasing, `CloudRepositoryChoice`) becomes
  one catalog sentence per case, keyed by the noun's key.
- Mixed sentences (frontend words + English backend sentence) use one
  convention: Chinese lead-in, full-width colon, the backend sentence
  unchanged — "无法同步：Unavailable for …".
- `sessionDeletion.ts` joins an outcome line ("2 local sessions deleted
  before deletion stopped…", English plural) to a reason paragraph. In the
  catalog it is one key per outcome (none deleted, n deleted with
  `_one`/`_other`) and one per reason, joined by the frontend.
- Enums printed raw get keys: a tool call's status (`{tool.status}`,
  `SessionDetailToolCall.tsx`), an agent's mode (the facts chip and
  `Run ${agent.mode}`, `SessionDetailEvent.tsx`, `SessionDetailHeader.tsx`),
  and `SESSION_AGENT_STATUS_LABELS` (`sessionDetailModel.ts:149`).

### Native

- `lib.rs:58-101`: build the menu from a small Rust table (en/zh, ~20
  labels, every predefined item given `Some(text)`); a `set_ui_language`
  command rebuilds it.
- `CFBundleLocalizations` = en, zh-Hans in the bundle's Info.plist, so macOS
  lists Mallard under per-app languages and its own panels (open dialogs)
  speak Chinese on a Chinese Mac. `NSAppleEventsUsageDescription` gets an
  `InfoPlist.strings` translation.

### Logs

- Lines stay English. The drawer's chrome, filters and the type tag (today
  the raw key, `LogPanel.tsx:123`) are translated. Log search keeps
  matching English text only — a Chinese search finds nothing; the search
  field's placeholder says so in Chinese.

## Hazards

- **Tests that read source text.** ~900 `readFileSync` assertions match raw
  JSX; ~143 match English copy and move with their area. ~25 of those are
  `doesNotMatch` — they would keep passing while checking nothing, so each
  is rewritten, not left. Heaviest: storage-catalog (57), cutover (29).
- **Width.** 18 `ch`-based widths and the path bar's action budget (floor
  580 + minWidth 824) are re-measured at the 11px Chinese floor. Han text
  breaks between any two characters where English cannot: the session
  timeline's gap pill (`.session-detail-event-gap`, a 24px rail) stacked
  "+1分钟" one character per line until it was given `white-space: nowrap`.
- **Partial Chinese.** zh-CN is added to the supported list only in the last
  step of each side, so no build ships a half-Chinese app.
- **Transcripts look like copy.** A reply that reads "Done." or "Pull first"
  is the agent's words. Every transcript container (Markdown, tool summary
  and payload, an agent's label, brief and report, commit subjects) is
  outside the catalog by construction; a test asserts the session page
  renders an English transcript unchanged with zh-CN on.
- **Skill names are ASCII.** The backend's name rule
  (`validate_component_name`, `storage_catalog/domain.rs:310`) refuses a
  skill folder named in Chinese, in an English sentence ("invalid skill name:
  '潮汐笔记'"); the Agent Skills format's own name rule is ASCII too. The refusal stays
  English like every backend error; descriptions are free text and show as
  written. Not an i18n change — noted because Chinese users will meet it.

## Steps

Each step keeps English output byte-identical and ends with the AGENTS.md
checks for what it touched, plus a Chinese render smoke test whose
missing-key handler throws.

Desktop (mallard-app):
1. **D1 Foundations.** i18next setup, preference, `<html lang>`, locale-aware
   formatters, `:lang(zh)` type rules, catalog parity type, test helpers, a
   guard test (TypeScript AST) that fails on English JSX text or literal
   `title`/`aria-label`/`placeholder` in migrated directories. Measure `tsc`
   with a generated 2.8k-key catalog to settle string keys vs selectors.
   WebKit font check with the offscreen WKWebView probe.
2. **D2 Shell.** Sidebar, path bar, app menu, Settings (Language row built
   but zh not yet offered), notices, updater, app-link sheet, Logs drawer.
3. **D3 Projects.** Sessions and Git timeline, session detail, project
   files, New session, project add and setup.
4. **D4 Reviews.** Push and Pull reviews, outcomes, restore plan; the
   copy-as-logic keys.
5. **D5 Storages.** List, settings, setup flows (Local, R2, Dropbox, Feishu,
   Mallard Cloud), Cloud project picker.
6. **D6 Library.** StorageCatalogPage, item pages, pet desk.
7. **D7 Backend labels** (if decision 3 holds): add the missing keys in
   Rust and `src/types.ts` together; frontend catalogs keyed by them.
8. **D8 On.** Native menu, Info.plist, `confirm()` labels; add zh-CN to the
   supported list; live Tauri pass on macOS (`-AppleLanguages '(zh-Hans)'`)
   and Windows (WebView2 language check).

Website (mallardInternal/cloudflare/managed-cloud):
1. **W1 Foundations** — the same pieces, boot-screen line, bar and ⋯ menu
   switch built but zh not offered (the switch renders nothing while English
   is the only language offered). Done: i18next 26.4.2 + react-i18next
   17.0.15 pinned; 9 namespaces in `web/src/i18n`; `{{count, number}}` formats
   like `toLocaleString()`; the guard is `scripts/i18n-guard.mjs` on oxc's
   parser through `vite` (TypeScript 7 has no script API); TypeScript 7
   rejects a missing, extra or unknown key.
2. **W2 Hub** — home, repository tabs, session page, release and versions,
   settings, invitations, reviews, profile.
3. **W3 Landing and docs** — headline, demo scene strings (`scene.ts`,
   `docs/art.js`), docs pages; the Privacy note.
4. **W4 On** — add zh-CN to the supported list; deploy to staging first.

## Safety and schema

No safety invariant changes. No manifest, `~/.mallard` or service schema
change: the language is a browser-storage preference on both sides. D7
changes Tauri DTOs (keys beside or instead of English labels) in Rust and
`src/types.ts` together; nothing persisted carries them except the default
storage names, which stay as saved.

## Verification

- Desktop: `npm run build`, `npm run test:frontend-integration`,
  `cargo test --manifest-path src-tauri/Cargo.toml --lib` (D7, D8).
- Website: `npm run typecheck`, `npm run test:web`, `npm test` (Worker
  unchanged).
- Screens: the specimen frames re-shot from the real catalogs in both
  languages and both themes.

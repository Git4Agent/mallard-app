import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "../src/App.css";
import "./i18n-zh-mockup.css";

/**
 * Mockup: Mallard in Simplified Chinese (2026-10-07), round 2. Owner
 * decisions: Push = 上传, Pull = 下载; direction A — a Language row under
 * Theme in Settings › Appearance; backend errors stay English; whatever the
 * user typed is never translated.
 *
 * Every frame is the shipped shell, live on a stand-in backend
 * (i18n-zh-live.html), with a specimen-only shim rewriting its English — the
 * layout is real, the copy is a draft. The Language row is clickable: it
 * reloads its frame in the language picked.
 *
 * `npm run dev`, then /mockups/i18n-zh-mockup.html
 *   ?lang=zh|en  ?font=cjk|default  ?theme=light|dark
 */

type Lang = "zh" | "en";

function readKnobs() {
  const query = new URLSearchParams(window.location.search);
  const pick = <T extends string>(key: string, allowed: readonly T[], fallback: T): T => {
    const value = query.get(key) as T | null;
    return value && allowed.includes(value) ? value : fallback;
  };
  return {
    lang: pick<Lang>("lang", ["zh", "en"], "zh"),
    font: pick("font", ["cjk", "default"] as const, "cjk"),
    theme: pick("theme", ["light", "dark"] as const, "light"),
  };
}

const TERMS: Array<[string, string]> = [
  ["Push", "上传"], ["Pull", "下载"], ["Session", "会话"], ["Repository", "仓库"], ["Storage", "存储"],
  ["Library", "资源库"], ["Subagent", "子代理"], ["Snapshot", "快照"],
];

interface Frame { key: string; title: string; params: string }

/** The shell at a real 1280×800 window; `small` shows it at 55% so two fit side by side. */
function Live({ params, tall, small, wrap }: { params: string; tall?: boolean; small?: boolean; wrap?: boolean }) {
  const page = wrap ? "i18n-zh-wrap" : "i18n-zh-live";
  return (
    <div className={small ? "zh-live small" : tall ? "zh-live tall" : "zh-live"}>
      <iframe key={params} src={`/mockups/${page}.html?${params}`} title={params} loading="lazy" />
    </div>
  );
}

interface WrapFrame { key: string; en: string; zh: string; params: string }
/* Screens other live harnesses already stand up, shown through i18n-zh-wrap. */
const WRAPPED: WrapFrame[] = [
  { key: "pull-sheet", en: "Open in Mallard · a session link", zh: "在 Mallard 中打开 · 会话链接", params: "h=session-pull-live&cold=session" },
  { key: "pull-session", en: "Pull review · one session", zh: "下载审阅 · 单个会话", params: "h=session-pull-live&cold=session&steps=Pull%20session%E2%80%A6" },
  { key: "pull-project", en: "Pull review · project", zh: "下载审阅 · 项目", params: "h=session-pull-live&steps=starts:Pull" },
  { key: "pull-forged", en: "Open in Mallard · broken link", zh: "在 Mallard 中打开 · 无效链接", params: "h=session-pull-live&cold=forged" },
  { key: "session-detail-unavailable", en: "Session page · unavailable", zh: "会话页面 · 不可用", params: "h=session-pull-live&state=synced&steps=starts:View%20session%20details" },
  { key: "files-tab", en: "Non-Git project · Files", zh: "非 Git 项目 · 文件", params: "h=push-files-live&steps=Files" },
  { key: "push-files", en: "Push review · sessions and files", zh: "上传审阅 · 会话与文件", params: "h=push-files-live&steps=starts:Push" },
  { key: "item-skill", en: "Library item · skill (content stays as written)", zh: "资源库条目 · Skill（内容保持原文）", params: "h=catalog-item-live&kind=skill&storage=cloud" },
  { key: "item-plugin", en: "Library item · plugin", zh: "资源库条目 · 插件", params: "h=catalog-item-live&kind=plugin&storage=cloud" },
  { key: "item-pet", en: "Library item · pet", zh: "资源库条目 · 宠物", params: "h=catalog-item-live&kind=pet&storage=vault" },
  { key: "library-add", en: "Library · Add resource", zh: "资源库 · 添加资源", params: "h=catalog-add-live" },
  { key: "cloud-project", en: "New project · From Mallard Cloud", zh: "新项目 · 来自 Mallard Cloud", params: "h=cloud-project-live" },
  { key: "storage-detail", en: "Storage pages · connected, refused", zh: "存储页面 · 已连接、被拒绝", params: "h=storage-locked-live" },
  { key: "link-project-new", en: "Open in Mallard · project not here", zh: "在 Mallard 中打开 · 本机没有的项目", params: "h=deep-link-live&cold=project-new" },
  { key: "link-item-new", en: "Open in Mallard · Library item", zh: "在 Mallard 中打开 · 资源库条目", params: "h=deep-link-live&cold=item-new" },
  { key: "notices-push", en: "Push and Pull notices (captions are the old specimen's)", zh: "上传与下载提示（说明文字来自旧样张）", params: "h=push-notices-live" },
  { key: "notices-session", en: "Session notes", zh: "会话提示", params: "h=session-notices-live" },
  { key: "notices-cloud", en: "Mallard Cloud notices", zh: "Mallard Cloud 提示", params: "h=cloud-gone-live" },
];

function Specimen() {
  const initial = readKnobs();
  const [lang, setLang] = useState<Lang>(initial.lang);
  const [font, setFont] = useState(initial.font);
  const [theme, setTheme] = useState(initial.theme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
    const query = new URLSearchParams();
    query.set("lang", lang); query.set("font", font); query.set("theme", theme);
    window.history.replaceState(null, "", `?${query}`);
  }, [lang, font, theme]);

  const seg = <V extends string>(value: V, current: V, set: (value: V) => void, label: string) => (
    <button key={value} type="button" className="spec-seg" aria-pressed={value === current} onClick={() => set(value)}>{label}</button>
  );
  const common = `font=${font}&theme=${theme}`;
  const settings: Frame[] = [
    { key: "system", title: "跟随系统 · the Mac prefers Chinese", params: `screen=settings&lang=zh&pref=system&${common}` },
    { key: "en", title: "English, picked", params: `screen=settings&lang=en&pref=en&${common}` },
  ];
  const flows: Frame[] = [
    { key: "storages", title: lang === "zh" ? "存储" : "Storages", params: `screen=storages&lang=${lang}&${common}` },
    { key: "storage-add", title: lang === "zh" ? "添加存储 · 选择类型" : "Add storage · type", params: `screen=storage-add&lang=${lang}&${common}` },
    { key: "storage-cloud", title: "Mallard Cloud", params: `screen=storage-cloud&lang=${lang}&${common}` },
    { key: "storage-r2", title: "Cloudflare R2", params: `screen=storage-r2&lang=${lang}&${common}` },
    { key: "storage-local", title: lang === "zh" ? "本地文件夹" : "Local folder", params: `screen=storage-local&lang=${lang}&${common}` },
    { key: "project-add", title: lang === "zh" ? "添加项目" : "Add project", params: `screen=project-add&lang=${lang}&${common}` },
    { key: "project-setup", title: lang === "zh" ? "项目设置" : "Project setup", params: `screen=project-setup&lang=${lang}&${common}` },
    { key: "library", title: lang === "zh" ? "资源库 · 空" : "Library · empty", params: `screen=library&lang=${lang}&${common}` },
    { key: "logs", title: lang === "zh" ? "日志 · 记录保持英文" : "Logs · lines stay English", params: `screen=logs&lang=${lang}&${common}` },
  ];
  const t = (en: string, zh: string) => (lang === "zh" ? zh : en);
  const steps: Frame[] = [
    { key: "storage-cloud-signin", title: t("Mallard Cloud · browser sign-in", "Mallard Cloud · 浏览器登录"), params: `screen=storage-cloud-signin&lang=${lang}&${common}` },
    { key: "storage-r2-create", title: t("R2 · Create new", "R2 · 新建"), params: `screen=storage-r2-create&lang=${lang}&${common}` },
    { key: "storage-r2-signin", title: t("R2 · Wrangler sign-in", "R2 · Wrangler 登录"), params: `screen=storage-r2-signin&lang=${lang}&${common}` },
    { key: "storage-r2-account", title: t("R2 · account", "R2 · 账户"), params: `screen=storage-r2-account&lang=${lang}&${common}` },
    { key: "storage-r2-bucket", title: t("R2 · bucket", "R2 · 存储桶"), params: `screen=storage-r2-bucket&lang=${lang}&${common}` },
    { key: "storage-r2-authorize", title: t("R2 · authorize key", "R2 · 授权密钥"), params: `screen=storage-r2-authorize&lang=${lang}&${common}` },
    { key: "storage-r2-existing", title: t("R2 · Use existing, scanned", "R2 · 使用已有，扫描结果"), params: `screen=storage-r2-existing&lang=${lang}&${common}` },
    { key: "storage-dropbox-signin", title: t("Dropbox · browser approval", "Dropbox · 浏览器批准"), params: `screen=storage-dropbox-signin&lang=${lang}&${common}` },
    { key: "storage-dropbox-folder", title: t("Dropbox · folder name", "Dropbox · 文件夹名称"), params: `screen=storage-dropbox-folder&lang=${lang}&${common}` },
    { key: "storage-local-ready", title: t("Local folder · tested", "本地文件夹 · 已测试"), params: `screen=storage-local-ready&lang=${lang}&${common}` },
  ];
  const cloud: Frame[] = [
    { key: "cloud-private", title: t("Mallard Cloud · Private", "Mallard Cloud · 私有"), params: `screen=cloud-private&lang=${lang}&${common}` },
    { key: "cloud-draft", title: t("Mallard Cloud · Draft past the public version", "Mallard Cloud · 草稿领先公开 Version"), params: `screen=cloud-draft&lang=${lang}&${common}` },
    { key: "cloud-public", title: t("Mallard Cloud · Public", "Mallard Cloud · 公开"), params: `screen=cloud-public&lang=${lang}&${common}` },
    { key: "cloud-review", title: t("Mallard Cloud · In review", "Mallard Cloud · 审核中"), params: `screen=cloud-review&lang=${lang}&${common}` },
    { key: "cloud-signin", title: t("Mallard Cloud · Signed out", "Mallard Cloud · 已退出登录"), params: `screen=cloud-signin&lang=${lang}&${common}` },
  ];
  const more: Frame[] = [
    { key: "storage-page", title: t("Storage page · repositories", "存储页面 · 仓库"), params: `screen=storage-page&lang=${lang}&${common}` },
    { key: "project-menu", title: t("Project menu", "项目菜单"), params: `screen=project-menu&lang=${lang}&${common}` },
    { key: "project-rename", title: t("Rename project", "重命名项目"), params: `screen=project-rename&lang=${lang}&${common}` },
    { key: "project-binding", title: t("Agent profile", "代理配置"), params: `screen=project-binding&lang=${lang}&${common}` },
    { key: "link-storage", title: t("Link storage", "关联存储"), params: `screen=link-storage&lang=${lang}&${common}` },
    { key: "session-info", title: t("Session info", "会话信息"), params: `screen=session-info&lang=${lang}&${common}` },
    { key: "new-session-menu", title: t("New session menu", "新会话菜单"), params: `screen=new-session-menu&lang=${lang}&${common}` },
    { key: "cloud-publish", title: t("Publish version", "发布 Version"), params: `screen=cloud-publish&lang=${lang}&${common}` },
    { key: "update", title: t("Update ready", "更新已就绪"), params: `screen=update&lang=${lang}&${common}` },
    { key: "startup", title: t("Starting up", "启动中"), params: `screen=startup&lang=${lang}&${common}` },
  ];
  const timeline: Frame[] = [
    { key: "session-detail", title: t("Session page · a Chinese transcript, every kind shown", "会话页面 · 中文对话，显示全部类型"), params: `screen=session-detail&lang=${lang}&${common}` },
    { key: "session-detail-tool", title: t("A failed tool call, opened (output stays as recorded)", "展开失败的工具调用（输出保持原样）"), params: `screen=session-detail-tool&lang=${lang}&${common}` },
    { key: "session-detail-agent", title: t("An agent row, opened: brief and report", "展开代理行：任务说明与报告"), params: `screen=session-detail-agent&lang=${lang}&${common}` },
    { key: "session-detail-child", title: t("The agent's own page", "代理自己的页面"), params: `screen=session-detail-child&lang=${lang}&${common}` },
    { key: "session-detail-en", title: t("An English transcript stays English", "英文对话保持英文"), params: `screen=session-detail-en&lang=${lang}&${common}` },
  ];
  const library: Frame[] = [
    { key: "library-scope", title: t("Library · Skills on R2, every state", "资源库 · R2 上的 Skill，各种状态"), params: `screen=library-scope&lang=${lang}&${common}` },
    { key: "library-push", title: t("Library Push review · a warning to acknowledge, one held", "资源库上传审阅 · 待确认的警告，一项等待下载"), params: `screen=library-push&lang=${lang}&${common}` },
    { key: "library-pushed", title: t("Library Push · stopped part-way (the reason stays English)", "资源库上传 · 中途停止（原因保持英文）"), params: `screen=library-pushed&lang=${lang}&${common}` },
    { key: "library-pull", title: t("Library Pull review · install, update, keep a copy, claimed elsewhere", "资源库下载审阅 · 安装、更新、保留副本、他处已固定"), params: `screen=library-pull&lang=${lang}&${common}` },
    { key: "library-pull-overwrite", title: t("Library Pull · Overwrite chosen, approval asked", "资源库下载 · 选择覆盖，需要确认"), params: `screen=library-pull-overwrite&lang=${lang}&${common}` },
    { key: "library-pulled", title: t("Library Pull · needs attention", "资源库下载 · 需要处理"), params: `screen=library-pulled&lang=${lang}&${common}` },
    { key: "library-fork", title: t("Fork a skill", "复刻 Skill"), params: `screen=library-fork&lang=${lang}&${common}` },
    { key: "library-fork-refused", title: t("Fork refused (the backend's sentence stays English)", "复刻被拒绝（后端原句保持英文）"), params: `screen=library-fork-refused&lang=${lang}&${common}` },
    { key: "library-cloud-push", title: t("On Mallard Cloud · Push creates a repository", "Mallard Cloud 上 · 上传会创建仓库"), params: `screen=library-cloud-push&lang=${lang}&${common}` },
    { key: "library-cloud-pull", title: t("On Mallard Cloud · Pull names each repository and version", "Mallard Cloud 上 · 下载列出每个仓库与版本"), params: `screen=library-cloud-pull&lang=${lang}&${common}` },
  ];
  const dialogs: Frame[] = [
    { key: "confirm-delete-session", title: t("Delete local session", "删除本地会话"), params: `screen=confirm-delete-session&lang=${lang}&${common}` },
    { key: "confirm-remove-project", title: t("Remove project", "移除项目"), params: `screen=confirm-remove-project&lang=${lang}&${common}` },
    { key: "confirm-unlink", title: t("Unlink storage", "取消关联存储"), params: `screen=confirm-unlink&lang=${lang}&${common}` },
    { key: "delete-session-in-use", title: t("Delete blocked · session in use", "删除受阻 · 会话正在使用"), params: `screen=delete-session-in-use&lang=${lang}&${common}` },
    { key: "delete-close-codex", title: t("Delete blocked · close Codex first", "删除受阻 · 先关闭 Codex"), params: `screen=delete-close-codex&lang=${lang}&${common}` },
    { key: "delete-codex-closed", title: t("Codex closed · try again", "Codex 已关闭 · 再试一次"), params: `screen=delete-codex-closed&lang=${lang}&${common}` },
    { key: "delete-all-stopped", title: t("Delete all · stopped after 2", "全部删除 · 删除 2 个后停止"), params: `screen=delete-all-stopped&lang=${lang}&${common}` },
  ];
  const screens: Frame[] = [
    { key: "sessions", title: lang === "zh" ? "项目 · 会话" : "Project · sessions", params: `screen=sessions&lang=${lang}&${common}` },
    { key: "push", title: lang === "zh" ? "上传审阅" : "Push review", params: `screen=push&lang=${lang}&${common}` },
    { key: "error", title: lang === "zh" ? "上传失败 · 后端错误保持英文" : "Push failed · backend error stays English", params: `screen=error&lang=${lang}&${common}` },
  ];

  return (
    <div className="spec">
      <h1 className="spec-title">Mallard · 简体中文</h1>
      <p className="zh-terms">
        {TERMS.map(([en, zh]) => <span key={en}><b>{zh}</b> {en}</span>)}
        <span className="zh-terms-keep">Version · Mallard · Mallard Cloud · Codex · Claude stay English</span>
      </p>
      <p className="zh-rules">
        <span>Never translated: what the user typed — names, titles, descriptions, commits, paths</span>
        <span>Stays English: backend errors and log lines</span>
      </p>
      <div className="spec-controls" role="group" aria-label="Specimen controls">
        <div className="spec-group">{seg<Lang>("zh", lang, setLang, "中文")}{seg<Lang>("en", lang, setLang, "English")}</div>
        <div className="spec-group">{seg("cjk", font, setFont, "CJK font stack")}{seg("default", font, setFont, "Font as shipped")}</div>
        <div className="spec-group">{seg("light", theme, setTheme, "Light")}{seg("dark", theme, setTheme, "Dark")}</div>
      </div>

      <h2 className="spec-h2">A · Language, under Theme <em className="zh-picked">picked</em></h2>
      <div className="zh-pair">
        {settings.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">Setup, storages, Library, Logs</h2>
      <div className="zh-flows">
        {flows.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">Storage setup, step by step</h2>
      <div className="zh-flows">
        {steps.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">More of the app</h2>
      <div className="zh-flows">
        {more.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">{t("Session page, the whole timeline", "会话页面，完整时间线")}</h2>
      <p className="zh-rules">
        <span>{t("Prompts, replies, tool commands and output, commit subjects, an agent's brief and report stay as recorded", "提示词、回复、工具命令与输出、提交说明、代理的任务说明与报告保持原样")}</span>
      </p>
      <div className="zh-flows">
        {timeline.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">{t("Library Push, Pull, and fork", "资源库上传、下载与复刻")}</h2>
      <div className="zh-flows">
        {library.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">{t("Native dialogs, drawn from what the app asks macOS", "原生对话框（按应用发给 macOS 的内容绘制）")}</h2>
      <div className="zh-flows">
        {dialogs.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">{t("From the other live harnesses", "来自其他实时样张")}</h2>
      <div className="zh-flows">
        {WRAPPED.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{lang === "zh" ? frame.zh : frame.en}</h3>
            <Live params={`${frame.params}&lang=${lang}&${common}`} small wrap />
          </section>
        ))}
      </div>

      <h2 className="spec-h2">The app, live</h2>
      <div className="zh-screens">
        {screens.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} tall />
          </section>
        ))}
      </div>
      <div className="zh-flows">
        {cloud.map((frame) => (
          <section key={frame.key} className="spec-cell">
            <h3 className="spec-h">{frame.title}</h3>
            <Live params={frame.params} small />
          </section>
        ))}
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<Specimen />);

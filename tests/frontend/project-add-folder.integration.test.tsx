import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import NewProjectFolderStep from "../../src/components/project-sync/NewProjectFolderStep";
import { compactProjectPath } from "../../src/components/project-sync/model";
import {
  compactFolderPath,
  folderChoice,
  folderVerdict,
  formatRecentStamp,
  isAppScratchFolder,
  readFolderInput,
  type FolderProbeState,
} from "../../src/components/project-sync/projectFolder";
import type { LocalProjectSummary, ProjectFolderProbe, ProviderDescriptor } from "../../src/types";

const HOME = "/home/you";
const compact = (path: string) => compactFolderPath(path, HOME);

const blog = {
  local_project_id: "project-blog",
  bundle_id: "bundle-blog",
  display_name: "blog",
  revision: 1,
  project_root: `${HOME}/code/blog`,
} as LocalProjectSummary;

const folder = (overrides: Partial<Extract<ProjectFolderProbe, { kind: "folder" }>> = {}): ProjectFolderProbe => ({
  kind: "folder",
  canonical_path: `${HOME}/code/app`,
  is_git_repository: true,
  existing_draft_id: null,
  existing_project_ids: [],
  ...overrides,
});

const settled = (path: string, probe: ProjectFolderProbe): FolderProbeState => ({ status: "done", path, probe });

test("the Folder field reads paths the way people paste them", () => {
  assert.deepEqual(readFolderInput("", HOME), { kind: "empty" });
  assert.deepEqual(readFolderInput("   ", HOME), { kind: "empty" });
  assert.deepEqual(readFolderInput("~", HOME), { kind: "path", path: HOME });
  assert.deepEqual(readFolderInput("~/code/app/", HOME), { kind: "path", path: `${HOME}/code/app` });
  // A terminal copy arrives quoted; a file manager's as a URL.
  assert.deepEqual(readFolderInput(`'${HOME}/code/my app'`, HOME), { kind: "path", path: `${HOME}/code/my app` });
  assert.deepEqual(readFolderInput(`file://${HOME}/code/my%20app`, HOME), { kind: "path", path: `${HOME}/code/my app` });
  assert.deepEqual(readFolderInput("file:///C:/work/app", "C:\\Users\\you"), { kind: "path", path: "C:/work/app" });
  // Windows drives and shares are paths too; a root keeps its separator.
  assert.deepEqual(readFolderInput("C:\\work\\app\\", "C:\\Users\\you"), { kind: "path", path: "C:\\work\\app" });
  assert.deepEqual(readFolderInput("C:\\", "C:\\Users\\you"), { kind: "path", path: "C:\\" });
  assert.deepEqual(readFolderInput("/", HOME), { kind: "path", path: "/" });
  assert.deepEqual(readFolderInput("\\\\server\\share\\app", null), { kind: "path", path: "\\\\server\\share\\app" });
  // Anything else searches the suggestions.
  assert.deepEqual(readFolderInput("mall", HOME), { kind: "search", query: "mall" });
  assert.deepEqual(readFolderInput("code/app", HOME), { kind: "search", query: "code/app" });

  assert.equal(compact(`${HOME}/code/app`), "~/code/app");
  assert.equal(compact(HOME), "~");
  assert.equal(compact("/home/youth/app"), "/home/youth/app");
  assert.equal(compactFolderPath("C:\\Users\\you\\code", "C:\\Users\\you"), "~/code");
  // Landing cards and the sidebar shorten Linux homes as they do macOS ones.
  assert.equal(compactProjectPath("/home/you/code/app"), "~/code/app");
  assert.equal(compactProjectPath("/Users/you/code/app"), "~/code/app");
  assert.equal(compactProjectPath("/srv/code/app"), "/srv/code/app");
});

test("the verdict line says what setup would make of the folder", () => {
  const path = `${HOME}/code/app`;
  const input = { kind: "path", path } as const;
  const context = { compact, projects: [blog] };
  const verdict = (probe: ProjectFolderProbe) => folderVerdict(input, settled(path, probe), context);

  // An empty field carries no sentence; the placeholder speaks for it.
  assert.equal(folderVerdict({ kind: "empty" }, { status: "idle" }, context), null);
  // A search on the New project step leaves the line to the list.
  assert.equal(folderVerdict({ kind: "search", query: "app" }, { status: "idle" }, context), null);
  assert.deepEqual(folderVerdict(input, { status: "checking", path }, context), { tone: "mute", message: "Checking", path: "~/code/app" });
  assert.deepEqual(verdict(folder()), { tone: "ok", message: "Git repository" });
  assert.deepEqual(verdict(folder({ is_git_repository: false })), { tone: "ok", message: "Folder · not a Git repository" });
  assert.deepEqual(
    verdict(folder({ canonical_path: `${HOME}/src/app` })),
    { tone: "ok", message: "Git repository · resolves to", path: "~/src/app" },
  );
  assert.deepEqual(verdict({ kind: "missing" }), { tone: "bad", message: "No folder at this path yet." });
  assert.deepEqual(verdict({ kind: "not_a_directory", parent: path }), {
    tone: "bad",
    message: "This is a file.",
    action: { label: "Use ~/code/app", value: "~/code/app" },
  });
  assert.match(verdict({ kind: "mallard_data", contains: true })!.message, /holds Mallard’s own data/);
  assert.match(verdict({ kind: "mallard_data", contains: false })!.message, /Mallard’s own data folder/);
  assert.equal(verdict({ kind: "unreadable", message: "denied" })!.tone, "warn");
  // A draft already here is resumed, and a project here is not a refusal.
  assert.deepEqual(verdict(folder({ existing_draft_id: "draft-1" })), { tone: "ok", message: "Setup already started for this folder." });
  assert.deepEqual(verdict(folder({ existing_project_ids: ["project-blog"] })), {
    tone: "ok",
    message: "Already added as “blog”.",
    action: { label: "Set it up with another agent profile", anotherProfile: true },
  });
  // A stale answer for an earlier path is never shown as this path's.
  assert.equal(folderVerdict(input, settled(`${HOME}/code`, folder()), context)!.message, "Checking");

  // On the draft's own page, its folder is simply a folder; another draft's
  // is not somewhere it can move, and a search is a path still to finish.
  const onPage = { ...context, draftId: "draft-1" };
  assert.deepEqual(folderVerdict(input, settled(path, folder({ existing_draft_id: "draft-1" })), onPage), { tone: "ok", message: "Git repository" });
  assert.equal(folderVerdict(input, settled(path, folder({ existing_draft_id: "draft-2" })), onPage)!.tone, "warn");
  assert.match(folderVerdict({ kind: "search", query: "app" }, { status: "idle" }, onPage)!.message, /Paths must be absolute/);
});

test("app scratch folders fold away and recent dates stay short", () => {
  assert.equal(isAppScratchFolder("/Users/you/Library/Application Support/Claude/scratch-workspaces/5654/app"), true);
  assert.equal(isAppScratchFolder("C:\\Users\\you\\AppData\\Local\\Temp\\work"), true);
  assert.equal(isAppScratchFolder("/home/you/.local/share/app/work"), true);
  assert.equal(isAppScratchFolder("/private/var/folders/rj/T/work"), true);
  assert.equal(isAppScratchFolder("/tmp/work"), true);
  assert.equal(isAppScratchFolder("/Users/you/Documents/Codex/2026-09-16/voice-chat"), false);
  assert.equal(isAppScratchFolder("/Users/you/code/app"), false);

  const now = Date.UTC(2026, 8, 25, 12, 0, 0);
  const at = (secondsAgo: number) => Math.floor(now / 1000) - secondsAgo;
  assert.equal(formatRecentStamp(at(20), now), "just now");
  assert.equal(formatRecentStamp(at(12 * 60), now), "12m ago");
  assert.equal(formatRecentStamp(at(3 * 3600), now), "3h ago");
  assert.doesNotMatch(formatRecentStamp(at(3 * 86_400), now), /2026|ago/);
  assert.match(formatRecentStamp(Date.UTC(2025, 5, 1) / 1000, now), /2025/);
});

test("choosing a folder resumes its draft, opens its project, or starts setup", () => {
  const path = `${HOME}/code/app`;
  assert.equal(folderChoice(null, path, [blog]), null);
  assert.equal(folderChoice({ kind: "missing" }, path, [blog]), null);
  assert.deepEqual(folderChoice(folder(), path, [blog]), { kind: "setup", path, resumes: false });
  assert.deepEqual(folderChoice(folder({ existing_draft_id: "draft-1", existing_project_ids: ["project-blog"] }), path, [blog]), {
    kind: "setup",
    path,
    resumes: true,
  });
  assert.deepEqual(folderChoice(folder({ existing_project_ids: ["project-blog"] }), path, [blog]), {
    kind: "open",
    projectId: "project-blog",
    name: "blog",
  });
});

test("New project opens at the Folder field and writes nothing until a folder is chosen", () => {
  const providers = [
    { key: "codex", display_name: "Codex", display_rank: 0, profile_label: "Codex home", setup_default: true },
  ] as unknown as ProviderDescriptor[];
  const html = renderToStaticMarkup(
    <NewProjectFolderStep
      workflowEpoch={1}
      isWorkflowCurrent={() => true}
      projects={[blog]}
      providers={providers}
      busy={false}
      onCancel={() => {}}
      onChoose={() => {}}
      onOpenProject={() => {}}
    />,
  );
  assert.match(html, /<h1 id="sync-new-project-title">New project<\/h1>/);
  // One band named Folder; the field's own label stays for screen readers.
  assert.match(html, /<div class="sync-card-heading"><div><strong>Folder<\/strong><\/div><\/div>/);
  assert.doesNotMatch(html, /<strong>Project<\/strong>/);
  assert.match(html, /class="sync-project-folder-field"[\s\S]*?<label[^>]*class="sync-visually-hidden"[^>]*>Folder<\/label>[\s\S]*?<input[^>]*autofocus/i);
  assert.match(html, /Browse…/);
  assert.match(html, /placeholder="Path, such as ~\/code\/app"/);
  assert.doesNotMatch(html, /sync-setup-path-hint/);
  // The recent folders load after mount; the first paint says so.
  assert.match(html, /class="sync-folder-choices-state"[\s\S]*?Finding recent folders…/);
  assert.match(html, /<button type="button" class="btn btn-primary" disabled="">Use this folder<\/button>/);

  const step = readFileSync("src/components/project-sync/NewProjectFolderStep.tsx", "utf8");
  const field = readFileSync("src/components/project-sync/ProjectFolderField.tsx", "utf8");
  // Typing only changes the text (and lets go of a picked row); a draft
  // comes from Enter, the button, a double-clicked row, or a Browse result.
  assert.match(step, /onChange=\{\(next\) => \{\s*setText\(next\);\s*setPicked\(null\);\s*\}\}/);
  // The list is the Repository band's option rows, with no scroller of its own.
  assert.match(step, /className="sync-bundle-match-options" role="radiogroup"/);
  assert.match(step, /role="radio"[\s\S]*?className=\{`sync-bundle-match-option\$\{active \? " active" : ""\}`\}/);
  assert.match(step, /<small className="sync-bundle-recommended">\{badge\}<\/small>/);
  assert.match(step, /const FIRST_FOLDERS = 6;/);
  // The fold toggles both ways, and folding keeps the picked row in view.
  assert.match(step, /aria-expanded=\{showAll\}\s*onClick=\{\(\) => setShowAll\(\(open\) => !open\)\}/);
  assert.match(step, /\{showAll \? "Show less" : `Show all \$\{listed\.length\}`\}/);
  assert.match(step, /pickedFolder && !firstRows\.includes\(pickedFolder\) \? \[\.\.\.firstRows, pickedFolder\] : firstRows/);
  // What the list leaves out is left out silently; scratch folders come back
  // only when the field searches for them or names them.
  assert.doesNotMatch(step, /Not shown|not usable|showScratch/);
  assert.match(step, /\(filtered \|\| !isAppScratchFolder\(folder\.path\)\)/);
  assert.doesNotMatch(step, /overflow|max-height/);
  assert.doesNotMatch(field, /onBlur/);
  assert.match(field, /catch \{\s*\/\/ Browse is the second way in[\s\S]*?setBrowseFailed\(true\)/);
});

test("the setup page's Folder moves the draft only on Enter or a Browse result", () => {
  const setup = readFileSync("src/components/project-sync/ProjectSetupWorkspace.tsx", "utf8");
  assert.doesNotMatch(setup, /changeProjectFolder|Change…/);
  assert.match(setup, /onChange=\{setFolderText\}/);
  // Only moveFolder edits project_root, and never into another draft's folder.
  assert.equal((setup.match(/project_root: path/g) ?? []).length, 1);
  assert.match(setup, /probe\.existing_draft_id && probe\.existing_draft_id !== draftId\)\) return;/);
  // Finish never sets up a folder still being typed.
  assert.match(setup, /&& !folderUncommitted;/);
  assert.match(setup, /Press Enter to use the new folder/);
});

// Finds English copy left in source: JSX text, string literals in copy-shaped
// attributes and object properties, and any other literal that reads as a
// sentence. Copy belongs in `src/i18n`; what stays English on purpose (a
// backend sentence compared against, a product name) carries a
// `// i18n-ignore: <why>` comment on its line or the line above.
//
//   node scripts/i18n-guard.mjs <file or directory>...
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript";

/** Attributes and properties whose value a person reads. */
const COPY_NAMES = new Set([
  "title", "aria-label", "aria-description", "aria-valuetext", "placeholder", "alt", "data-tooltip",
  "label", "ariaLabel", "message", "detail", "description", "hint", "summary", "menuBanner", "heading",
  "caption", "lead", "note", "subtitle", "helper", "tooltip", "emptyText", "verb", "noun", "text", "menuLabel", "subject", "okLabel", "cancelLabel",
]);
/** Attributes that hold identifiers, not prose. */
const CODE_ATTRIBUTES = new Set(["className", "key", "id", "role", "type", "href", "src", "name", "htmlFor", "rel", "target", "lang", "dir", "inputMode", "autoComplete", "method"]);
/** Calls whose string arguments are not shown to anyone, or already translated. */
const SILENT_CALLS = new Set(["t", "useTranslation", "invoke", "listen", "emit", "querySelector", "querySelectorAll", "closest", "matchMedia", "getItem", "setItem", "removeItem", "addEventListener", "removeEventListener", "startsWith", "endsWith", "includes", "test", "split", "join", "replace", "match", "getPropertyValue", "setProperty", "createElement", "toLocaleString", "toLocaleDateString", "toLocaleTimeString", "DateTimeFormat", "NumberFormat", "RegExp"]);
/** What may stay as it is: product names, the agents' own tool names, units,
 *  paths and symbols. "Version" stays English by the owner's decision. */
const ALLOWED = /^(Mallard|Mallard Cloud|Codex|Claude|Git|GitHub|R2|Cloudflare R2|S3|Dropbox|Feishu|Lark|Wrangler|macOS|Windows|Linux|Finder|Terminal|Bash|Read|Edit|Write|Grep|Glob|Task|Version|MB|KB|GB|B|OK|URL|API|ID|JSON|CLI|PNG|SVG|MP4|PDF|SHA-256|v\d.*|~?\/\S*|[\s\W\d]*)$/;
/** Identifiers: icon names, snake_case keys, language tags, dotted paths. */
const IDENTIFIER = /^[A-Za-z0-9]+([_.:\/-][A-Za-z0-9]+)+$/;

const kept = (text) => ALLOWED.test(text.trim()) || IDENTIFIER.test(text.trim());
const sentence = (text) => /[A-Za-z]{2,}\s+[A-Za-z]{2,}/.test(text) && !kept(text);
const wordy = (text) => /[A-Za-z]{2,}/.test(text) && !kept(text);

function calleeName(call) {
  const expression = call.expression;
  if (ts.isIdentifier(expression)) return expression.text;
  if (ts.isPropertyAccessExpression(expression)) return expression.name.text;
  if (ts.isNewExpression?.(call) && ts.isIdentifier(call.expression)) return call.expression.text;
  return "";
}

function literalText(node) {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isTemplateExpression(node)) return [node.head.text, ...node.templateSpans.map((span) => span.literal.text)].join(" ");
  return null;
}

function ignored(source, node) {
  const { line } = source.getLineAndCharacterOfPosition(node.getStart(source));
  const lines = source.text.split("\n");
  return /i18n-ignore/.test(lines[line] ?? "") || /i18n-ignore/.test(lines[line - 1] ?? "");
}

/** Where a literal sits decides whether it is copy. */
function context(node) {
  for (let parent = node.parent; parent; node = parent, parent = parent.parent) {
    if (ts.isImportDeclaration(parent) || ts.isExportDeclaration(parent) || ts.isLiteralTypeNode(parent)) return "code";
    if (ts.isJsxAttribute(parent)) {
      const name = parent.name.getText();
      if (COPY_NAMES.has(name)) return "copy";
      if (CODE_ATTRIBUTES.has(name) || name.startsWith("data-") || name === "style") return "code";
      return "other";
    }
    if (ts.isPropertyAssignment(parent) && parent.initializer === node) {
      const name = parent.name.getText().replace(/^["']|["']$/g, "");
      return COPY_NAMES.has(name) ? "copy" : "other";
    }
    if (ts.isCallExpression(parent) || ts.isNewExpression(parent)) {
      const name = ts.isCallExpression(parent) ? calleeName(parent) : parent.expression.getText();
      if (SILENT_CALLS.has(name) || name.startsWith("console")) return "code";
      return "other";
    }
    if (ts.isBinaryExpression(parent) && /^(===|!==|==|!=)$/.test(parent.operatorToken.getText())) return "code";
    if (ts.isCaseClause(parent) || ts.isElementAccessExpression(parent)) return "code";
    if (ts.isJsxExpression(parent) || ts.isConditionalExpression(parent) || ts.isParenthesizedExpression(parent)
      || ts.isBinaryExpression(parent) || ts.isTemplateSpan(parent) || ts.isTemplateExpression(parent)) continue;
    return "other";
  }
  return "other";
}

export function findUntranslatedCopy(files) {
  const findings = [];
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const report = (node, copy) => {
      if (ignored(source, node)) return;
      const { line, character } = source.getLineAndCharacterOfPosition(node.getStart(source));
      findings.push({ file, line: line + 1, column: character + 1, text: copy.replace(/\s+/g, " ").trim() });
    };
    const visit = (node) => {
      if (ts.isJsxText(node)) {
        if (wordy(node.text)) report(node, node.text);
        return;
      }
      const copy = literalText(node);
      if (copy !== null) {
        const where = context(node);
        // A template that sets a word against a value (`${n} file${s}`) is copy
        // even with one word: identifiers never hold a space.
        const spacedWord = ts.isTemplateExpression(node)
          && [node.head.text, ...node.templateSpans.map((span) => span.literal.text)].some((part) => /(^|\s)[A-Za-z]{2,}(\s|$)/.test(part) && /\s/.test(part) && !kept(part));
        if (where === "copy" ? wordy(copy) : where === "other" && (sentence(copy) || spacedWord)) report(node, copy);
        if (ts.isTemplateExpression(node)) node.templateSpans.forEach((span) => visit(span.expression));
        return;
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  return findings;
}

export function sourceFiles(paths) {
  return paths.flatMap((path) => statSync(path).isDirectory()
    ? readdirSync(path).flatMap((entry) => sourceFiles([join(path, entry)]))
    : /\.(ts|tsx)$/.test(path) && !path.endsWith(".d.ts") ? [path] : []);
}

if (process.argv[1]?.endsWith("i18n-guard.mjs")) {
  const findings = findUntranslatedCopy(sourceFiles(process.argv.slice(2)));
  for (const finding of findings) console.log(`${finding.file}:${finding.line}:${finding.column}  ${finding.text}`);
  console.log(`${findings.length} untranslated`);
  process.exitCode = findings.length ? 1 : 0;
}

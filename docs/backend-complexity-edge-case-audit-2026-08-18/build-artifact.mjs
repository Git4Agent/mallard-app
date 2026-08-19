import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const generatedAt = new Date().toISOString();
const commit = "cc6478e";
const repositoryUrl = "https://github.com/Git4Agent/mallard-app";
const sourceRoot = `${repositoryUrl}/blob/${commit}`;

function code(path, lines, label = `${path}:${lines}`) {
  const anchor = String(lines).replace(/^(\d+)-(\d+)$/, "L$1-L$2").replace(/^(\d+)$/, "L$1");
  return `[\`${label}\`](${sourceRoot}/${path}#${anchor})`;
}

function sqlLiteral(value) {
  if (typeof value === "number") {
    return String(value);
  }
  return `'${String(value).replaceAll("'", "''")}'`;
}

function snapshotValuesSql(rows, columns, orderBy) {
  const columnList = columns.map((column) => `"${column}"`).join(", ");
  const values = rows
    .map((row) => `(${columns.map((column) => sqlLiteral(row[column])).join(", ")})`)
    .join(",\n  ");
  return `WITH snapshot(${columnList}) AS (\n  VALUES ${values}\n) SELECT * FROM snapshot${orderBy ? ` ORDER BY ${orderBy}` : ""}`;
}

function tableSource({ id, label, href, rows, columns, orderBy, description }) {
  return {
    id,
    label,
    href,
    query: {
      engine: "SQLite-compatible VALUES query",
      language: "sql",
      description,
      sql: snapshotValuesSql(rows, columns, orderBy),
      executed_at: generatedAt,
      filters: ["Bounded reviewed snapshot", `Committed backend evidence baseline ${commit}`],
      tables_used: [],
    },
  };
}

const findings = [
  {
    rank: 1,
    id: "BACK-001",
    priority: "P0",
    confidence: "High",
    class: "Integrity / false proof",
    finding: "Rewrite-plus-append is classified as an append without proving the accepted prefix",
    consequence: "Push can publish and Pull can adopt rewritten accepted history",
    disposition: "Fix before simplification work",
  },
  {
    rank: 2,
    id: "BACK-002",
    priority: "P1",
    confidence: "High",
    class: "Silent state loss",
    finding: "Updating an existing project link reconstructs the aggregate and drops all conversation checkpoints",
    consequence: "Reviewed acceptance and physical mapping evidence disappear",
    disposition: "Mutate only pinned on the existing aggregate",
  },
  {
    rank: 3,
    id: "BACK-003",
    priority: "P1",
    confidence: "High",
    class: "Crash consistency",
    finding: "Remote-bundle identity reassignment is an incomplete ad hoc multi-document transaction",
    consequence: "A crash/failure can commit the new project identity without an active binding",
    disposition: "Remove the special route or journal it through the setup transaction",
  },
  {
    rank: 4,
    id: "BACK-004",
    priority: "P1",
    confidence: "High evidence / medium policy",
    class: "Over-detection / mutation",
    finding: "Codex path portability rewrites every matching string and object key, including conversation content",
    consequence: "Prose, code, tool arguments, keys, or sibling paths can change or block sync",
    disposition: "Restrict remapping to authoritative structural path fields",
  },
  {
    rank: 5,
    id: "BACK-005",
    priority: "P1",
    confidence: "High",
    class: "Codec correctness",
    finding: "Claude analysis records only the first cwd and permits mixed-root transcripts",
    consequence: "Relocation can leave later absolute roots unchanged while validation passes",
    disposition: "Require and re-prove one structural cwd across the transcript",
  },
  {
    rank: 6,
    id: "BACK-006",
    priority: "P1",
    confidence: "High",
    class: "External command safety",
    finding: "Plugin plans persist executable argv while provider inventory parsing is permissive where absence is security-relevant",
    consequence: "Broad probing gives false assurance; stale/tampered vectors remain executable plan state",
    disposition: "Persist semantic intent, parse exact schemas, rebuild argv in the adapter",
  },
  {
    rank: 7,
    id: "BACK-007",
    priority: "P1",
    confidence: "High",
    class: "False validation",
    finding: "Pet atlas validation accepts truncated header-like PNG/WebP data",
    consequence: "Corrupt, non-decodable resources can be published as valid",
    disposition: "Use a maintained bounded decoder/container validator",
  },
  {
    rank: 8,
    id: "BACK-008",
    priority: "P1",
    confidence: "High",
    class: "Pathological remote cost",
    finding: "Bundle and Library discovery scan payload/history namespaces to locate repository tags",
    consequence: "Unrelated object volume can exceed the one-million-key guard and break discovery",
    disposition: "List one flat closed discovery-index prefix",
  },
  {
    rank: 9,
    id: "BACK-009",
    priority: "P1",
    confidence: "High",
    class: "Aggregate bound failure",
    finding: "Catalog plans are bounded per file but unbounded as a directory",
    consequence: "Routine operations may parse an unbounded count of files, each allowed up to 64 MiB",
    disposition: "Use bounded active slots and prune expired/superseded plans",
  },
  {
    rank: 10,
    id: "BACK-010",
    priority: "P1",
    confidence: "High",
    class: "Over-detection / availability",
    finding: "Push rejects any blocked discovered conversation before evaluating the reviewed selection",
    consequence: "An unrelated unselected thread can block a healthy thread or project-file-only Push",
    disposition: "Fail closed only for selected actions",
  },
  {
    rank: 11,
    id: "BACK-011",
    priority: "P1",
    confidence: "High",
    class: "Over-detection / duplicate scan",
    finding: "Codex path readiness is an operation-global Push/Pull gate and then discovery runs again",
    consequence: "Corrupt unrelated Codex state can block generic-only or Claude-only work",
    disposition: "Attach ownership evidence to selected Codex actions",
  },
  {
    rank: 12,
    id: "BACK-012",
    priority: "P1",
    confidence: "High",
    class: "Conflicting authority",
    finding: "ProjectSync and Catalog maintain independent identities and persistence for the same provider homes",
    consequence: "Canonical drift, deletion, overlap, and revision policy can disagree",
    disposition: "One shared machine-local provider-profile authority",
  },
  {
    rank: 13,
    id: "BACK-013",
    priority: "P1",
    confidence: "High",
    class: "Identifier correctness / process cost",
    finding: "The local UUIDv7 fallback randomizes the timestamp field; the preferred allocator launches a disposable app-server",
    consequence: "Generated IDs are not RFC 9562 UUIDv7 and allocation carries avoidable process failure modes",
    disposition: "Generate a standards-compliant local UUIDv7, or use v4 if the provider permits it",
  },
  {
    rank: 14,
    id: "BACK-014",
    priority: "P1",
    confidence: "High",
    class: "Ambiguous ownership",
    finding: "Claude memory bucket ownership is last-writer-wins when sessions report different project-relative cwd values",
    consequence: "Shared memory can attach to the wrong project subtree",
    disposition: "Aggregate candidates and block/skip ambiguous buckets",
  },
  {
    rank: 15,
    id: "BACK-015",
    priority: "P2",
    confidence: "High",
    class: "Over-detection / parser reinvention",
    finding: "Project-content scanning treats generic directory names as credential paths and implements a partial ignore dialect",
    consequence: "Legitimate non-Git trees such as src/plugins can be blocked; ignore behavior diverges from Git",
    disposition: "Separate deny lists and use an established ignore parser",
  },
  {
    rank: 16,
    id: "BACK-016",
    priority: "P2",
    confidence: "High",
    class: "Dead optimization",
    finding: "The candidate-pin cache is deleted by required production rebaseline before the next review can reuse it",
    consequence: "Cache keys, invalidation, metrics, and tests maintain a benefit the shipped path cannot realize",
    disposition: "Delete candidate-pin reuse; retain Activity observation caching",
  },
  {
    rank: 17,
    id: "BACK-017",
    priority: "P2",
    confidence: "High",
    class: "Frozen dead surface",
    finding: "A historical freeze protects 94 Tauri commands despite numerous routes having no current consumer",
    consequence: "Dormant DTOs, handlers, validation, and tests prevent clean cutover",
    disposition: "Replace exact-name freeze with registration/reachability parity",
  },
  {
    rank: 18,
    id: "BACK-018",
    priority: "P2",
    confidence: "High",
    class: "Layering inversion",
    finding: "Non-Tauri service modules import and call the Tauri command modules that instantiate them",
    consequence: "Two behavioral entry paths exist; tests can bypass production preconditions",
    disposition: "Move use cases into application services and make commands thin adapters",
  },
  {
    rank: 19,
    id: "BACK-019",
    priority: "P2",
    confidence: "High",
    class: "Safety primitive drift",
    finding: "Catalog reimplements atomic persistence and no-follow primitives with weaker checks",
    consequence: "Hard-link, permission, and directory-fsync behavior differs across repositories",
    disposition: "Share the strongest safe_fs primitives; keep policy validation local",
  },
  {
    rank: 20,
    id: "BACK-020",
    priority: "P2",
    confidence: "Medium-high",
    class: "Journal/state multiplication",
    finding: "Conversation journals clone whole shared documents per action; Catalog splits one consistency domain across six documents",
    consequence: "State and recovery grow with actions and schema size, producing avoidable partial states",
    disposition: "One batch transaction per operation and one atomic Catalog state document",
  },
  {
    rank: 21,
    id: "BACK-021",
    priority: "P2",
    confidence: "High",
    class: "Unused/speculative state",
    finding: "Both remote protocols write unread commit objects; plugin/capability types encode states production cannot create",
    consequence: "Schema, objects, validation, and tests exist without a reader or product branch",
    disposition: "Remove in one schema cutover",
  },
  {
    rank: 22,
    id: "BACK-022",
    priority: "P2",
    confidence: "High",
    class: "Repeated work",
    finding: "Session detail, capture, plugin apply, config checks, continuity probes, and remote listing repeat full reads/scans",
    consequence: "Large transcripts and repositories amplify CPU, heap, subprocess, and network work",
    disposition: "Return typed combined snapshots and reuse one authoritative read per operation",
  },
  {
    rank: 23,
    id: "BACK-023",
    priority: "P2",
    confidence: "High",
    class: "Verification drift",
    finding: "The named backend gate omits extracted-crate tests and the formal model pins obsolete codec vectors",
    consequence: "Green gates do not cover the current safety kernel or detect model/implementation drift",
    disposition: "Run workspace lib tests and derive/share one golden-vector source",
  },
  {
    rank: 24,
    id: "BACK-024",
    priority: "P2",
    confidence: "High evidence / medium policy",
    class: "Over-attribution / redaction",
    finding: "Temporal Git mapping is labeled checkpoint and substring redaction rewrites near-prefix ordinary text",
    consequence: "UI provenance is overstated and visible conversation content can be corrupted",
    disposition: "Use explicit commit evidence and path-component-aware redaction",
  },
  {
    rank: 25,
    id: "BACK-025",
    priority: "P3",
    confidence: "Medium",
    class: "Product-dependent complexity",
    finding: "Activity JSONL, frozen Codex 0.147 projection, and managed R2 provisioning are large subsystems",
    consequence: "They dominate maintenance but may implement intentional product contracts",
    disposition: "Decide product support first; do not opportunistically delete",
  },
];

// Chart contract: answer "How is the action queue distributed by priority?"
// with a single-series categorical comparison. Four reviewed categories are
// sufficient for a bar chart; the semantic priority order is preserved, bars
// start at zero, color carries no category meaning, and exact values remain in
// the chart's source table. The chart is rendered only inside report.html.
const priorityCounts = [
  { priority: "P0", count: 1, meaning: "Immediate integrity repair" },
  { priority: "P1", count: 13, meaning: "Correctness, crash consistency, or normal-operation boundary" },
  { priority: "P2", count: 10, meaning: "Bounded simplification, performance, or maintainability" },
  { priority: "P3", count: 1, meaning: "Product-support decision before refactor" },
];

const edgeCases = [
  {
    id: "E01",
    area: "Conversation relation",
    trigger: "Accepted A,B(old); replace B with equal-length B(new); append C",
    current: "Outstanding → Publish/MaterializeCloud",
    required: "Divergent unless accepted-boundary extension is proved",
    priority: "P0",
  },
  {
    id: "E02",
    area: "Codex cursor",
    trigger: "Same-length edit before the trailing 4 KiB probe, then append",
    current: "Cursor may return trusted Extended from the old accumulator",
    required: "Full accepted-range proof or untrusted/full reparse",
    priority: "P0",
  },
  {
    id: "E03",
    area: "Project link",
    trigger: "Save an existing link with non-empty conversation checkpoints",
    current: "Replacement aggregate has an empty checkpoint map",
    required: "Every field remains byte-for-byte stable except pinned",
    priority: "P1",
  },
  {
    id: "E04",
    area: "Remote connect",
    trigger: "Failure after config identity commit and before active-binding recreation",
    current: "New bundle identity may persist without the active binding",
    required: "Restart reaches complete old or complete new fixed point",
    priority: "P1",
  },
  {
    id: "E05",
    area: "Push selection",
    trigger: "One healthy selected thread plus one blocked unselected thread",
    current: "Whole Push fails before selection is resolved",
    required: "Healthy action publishes; blocked row is held back",
    priority: "P1",
  },
  {
    id: "E06",
    area: "Provider isolation",
    trigger: "Claude-only or file-only operation with corrupt unrelated Codex assignment state",
    current: "Global Codex readiness gate blocks the operation",
    required: "Only selected Codex actions require Codex ownership evidence",
    priority: "P1",
  },
  {
    id: "E07",
    area: "Project content",
    trigger: "Non-Git project contains a legitimate plugins or repos subdirectory under source",
    current: "Generic component name is reported as a credential/private-key path",
    required: "Provider-home exclusions do not become project credential blocks",
    priority: "P2",
  },
  {
    id: "E08",
    area: "Plugin inventory",
    trigger: "Provider returns {}, missing required maps, or a changed top-level schema",
    current: "Parser can interpret unsupported schema as an empty inventory",
    required: "Typed unsupported-schema blocker",
    priority: "P1",
  },
  {
    id: "E09",
    area: "Plugin apply",
    trigger: "Persisted plan argv is tampered or becomes stale",
    current: "Stored vectors are executable state",
    required: "Adapter reconstructs allowlisted argv from reviewed semantic intent",
    priority: "P1",
  },
  {
    id: "E10",
    area: "Pet atlas",
    trigger: "PNG/WebP contains the expected header/dimensions but no complete image payload",
    current: "Header-only parser accepts it",
    required: "Complete bounded container/decode validation rejects it",
    priority: "P1",
  },
  {
    id: "E11",
    area: "Remote discovery",
    trigger: "Payload/history objects exceed the discovery scan ceiling while repository count is small",
    current: "Repository discovery fails before reaching tags",
    required: "List only a flat closed tag/index prefix",
    priority: "P1",
  },
  {
    id: "E12",
    area: "Catalog plan storage",
    trigger: "Thousands of expired or crash-abandoned 64 MiB plan files",
    current: "List/delete/prepare paths enumerate and parse the directory",
    required: "Small hard entry/byte limits plus pruning and active slots",
    priority: "P1",
  },
  {
    id: "E13",
    area: "Provider profiles",
    trigger: "A list/read call runs when a default provider home exists but no profile row does",
    current: "Listing writes state and increments revisions",
    required: "Read-only suggestions or explicit bootstrap",
    priority: "P2",
  },
  {
    id: "E14",
    area: "Claude memory",
    trigger: "Two owned sessions share one memory bucket but report . and sub as cwd",
    current: "Later map insertion wins",
    required: "Ambiguity is visible and no project target is guessed",
    priority: "P1",
  },
  {
    id: "E15",
    area: "Thread ID",
    trigger: "Codex app-server allocation fails and local fallback runs",
    current: "Random bits occupy UUIDv7's required 48-bit timestamp field",
    required: "RFC 9562 timestamp layout or an explicitly supported UUIDv4",
    priority: "P1",
  },
  {
    id: "E16",
    area: "Session redaction",
    trigger: "A project-root token appears as the prefix of a longer ordinary-language token",
    current: "Blind substring replacement produces .lication",
    required: "Exact path/component-aware redaction",
    priority: "P2",
  },
  {
    id: "E17",
    area: "Conversation journal",
    trigger: "One review applies N conversations against large config/binding documents",
    current: "N evolving before/after copies of both shared documents are journaled",
    required: "Per-target entries plus one shared before/after transaction",
    priority: "P2",
  },
  {
    id: "E18",
    area: "Apply-time config",
    trigger: "Config becomes unreadable after review when a conversation had no checkpoint",
    current: "load_config().ok() can collapse persistence failure into None",
    required: "One authoritative batch load; persistence error remains an error",
    priority: "P2",
  },
  {
    id: "E19",
    area: "Ignore semantics",
    trigger: "Escaped spaces, escaped hash/exclamation markers, or bracket classes in ignore files",
    current: "Custom matcher implements only a documented-looking subset",
    required: "Established ignore semantics or an explicitly narrower product contract",
    priority: "P2",
  },
  {
    id: "E20",
    area: "Remote listing",
    trigger: "First UI page in a repository with thousands of bundles and one corrupt distant manifest",
    current: "All bundles are inspected; distant corruption can fail the first page",
    required: "Page summaries first; inspect only the selected bundle",
    priority: "P2",
  },
];

const priorStatus = [
  { item: "Recursive/synthesized provider metadata detection", status: "Fixed", evidence: "Exact adapter-owned complete envelopes for Codex and Claude" },
  { item: "Claude sidecar substring ownership", status: "Fixed", evidence: "Exact sidecar grammar and ambiguity rejection" },
  { item: "Activity Git history loaded twice", status: "Fixed", evidence: "Current Activity path loads one first-parent history" },
  { item: "Single Library resource scans all Library state", status: "Fixed", evidence: "Resource-filtered inventory and batched link inspection" },
  { item: "Retired storage exclusions/resource-kind fallbacks", status: "Fixed", evidence: "Old field rejected; current unions are exhaustive" },
  { item: "Overdetermined conversation reconciliation", status: "Partially fixed", evidence: "Central relation/policy and grouped pins landed; accepted-prefix proof is still missing" },
  { item: "Repeated provider discovery/parsing", status: "Partially fixed", evidence: "Capture shares bytes and Activity shares reads; detail/plugin/global gates still repeat work" },
  { item: "Mixed Claude cwd relocation", status: "Open", evidence: "Only the first structural cwd is recorded and re-proved" },
  { item: "Formal model/vector drift", status: "Open", evidence: "Python and Rust pin different current codec vectors; model is not gated" },
  { item: "Custom ignore dialect", status: "Open", evidence: "Recursive DP glob implementation remains" },
  { item: "Profile list mutation/path guessing", status: "Open", evidence: "List can persist defaults; probe can silently select a child home" },
  { item: "Unused Tauri routes and historical freeze", status: "Open", evidence: "94 commands remain registered; dormant routes remain" },
  { item: "Temporary crate shims/controller concentration", status: "Open", evidence: "Temporary facade and 22.9k-line command controller remain" },
  { item: "Backend gate omits extracted crates", status: "Open", evidence: "Named backend script still tests only the Tauri lib plus Catalog E2E" },
  { item: "Implicit old Codex format support", status: "Product decision", evidence: "Frozen 0.147 projection and broad legacy normalization remain" },
];

const verification = [
  {
    check: "cargo test --workspace --lib",
    result: "PASS",
    coverage: "484 tests: Tauri 187, domain 44, infrastructure 219, application 26, CLI 8",
    caveat: "Executed against the current dirty workspace; findings are anchored to committed HEAD",
  },
  {
    check: "npm run test:backend-integration",
    result: "PASS",
    coverage: "187 Tauri lib tests plus 1 storage-catalog E2E",
    caveat: "Does not execute 289 domain/infrastructure/application lib tests",
  },
  {
    check: "Python portable-tail conflict model verifier",
    result: "PASS (stale)",
    coverage: "282 states, 312 transitions, 7 expected unsafe negative controls",
    caveat: "Python golden vector differs from current Rust codec and script is not wired into package gates",
  },
  {
    check: "Static command/reachability inventory",
    result: "COMPLETE",
    coverage: "94 registered Tauri commands: 69 ProjectSync, 20 Catalog, 5 Activity",
    caveat: "Reachability is from current source callsites, not dynamic plugin invocation",
  },
  {
    check: "Real provider homes / remote stores",
    result: "NOT RUN",
    coverage: "No real Codex, Claude, Mallard, S3, or R2 state was touched",
    caveat: "Performance and provider-schema risks require fixture/load tests before sizing production impact",
  },
];

const sources = [
  {
    id: "repo_audit",
    label: `Mallard backend source at ${commit}`,
    href: `${repositoryUrl}/tree/${commit}`,
    query: {
      engine: "git + ripgrep",
      language: "shell",
      description: "Read-only source inspection, callsite tracing, line counting, and contract comparison across committed Rust source, tests, and package scripts.",
      executed_at: generatedAt,
      filters: [
        "Backend Rust, Tauri command boundary, local persistence, provider codecs, storage protocols, and backend test gates",
        "Committed HEAD cc6478e used as the evidence baseline",
        "Uncommitted working-tree changes excluded from finding evidence",
        "Frontend implementation excluded except for reachability checks",
      ],
    },
  },
  {
    id: "test_runs",
    label: "Local verification commands",
    href: `${repositoryUrl}/blob/${commit}/package.json`,
    query: {
      engine: "Cargo and npm",
      language: "shell",
      description: "Workspace Rust library suite; named backend integration suite; Python portable-tail conflict model verifier.",
      executed_at: generatedAt,
      filters: ["No real provider home or Mallard state", "No external remote store"],
    },
  },
  {
    id: "prior_audits",
    label: "2026-08-17 complexity audits",
    href: `${repositoryUrl}/tree/${commit}/docs`,
    query: {
      engine: "Repository documents",
      language: "markdown",
      description: "Prior findings were treated as hypotheses and revalidated against current implementation rather than accepted as authority.",
      executed_at: generatedAt,
    },
  },
  {
    id: "rfc9562",
    label: "RFC 9562 — UUID Version 7",
    href: "https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7",
    query: {
      engine: "IETF RFC Editor",
      language: "text",
      description: "Primary standard defining UUIDv7's 48-bit Unix-millisecond timestamp and remaining random/monotonic fields.",
      executed_at: generatedAt,
    },
  },
];

const blocks = [
  {
    id: "title",
    type: "markdown",
    layout: "full",
    body: "# Mallard Backend Complexity, Over-Detection, and Edge-Case Audit",
  },
  {
    id: "technical_summary",
    type: "markdown",
    layout: "full",
    body: `## Technical Summary

The backend is **safety-heavy but not uniformly safety-proving**. Its strongest pieces—immutable content-addressed publication, head compare-and-swap, bounded/no-follow traversal, exact target digests, plan/apply separation, backups, and crash journals—should remain. The highest-risk defect is instead a missing proof hidden by that surrounding machinery: ${code("crates/mallard-domain/src/conversation_tail_relation.rs", "179-195")} treats a longer tail as an append without proving that the previously accepted canonical prefix is still present. A rewrite-plus-append can therefore become **Publish** or **MaterializeCloud**. This is P0 because the system can consistently validate the wrong reconciliation decision.

Three other P1 findings are immediately actionable: saving an existing project link silently drops conversation checkpoints; remote-bundle identity reassignment can commit half of a cross-document change; and several validators either inspect too broadly or prove too little—arbitrary Codex content is rewritten for path portability, mixed Claude roots pass relocation, plugin schemas can degrade to an empty inventory, and truncated pet atlases pass header-only validation.

The clearest over-engineering is concentrated rather than universal: a 22.9k-line ProjectSync command controller; application services that call back into command modules; 94 frozen Tauri commands with dormant routes; a candidate-pin cache invalidated before production reuse; two provider-profile authorities; six Catalog state documents under one lock; repeated discovery/parser passes; and remote commit/type structures with no reader. These are good deletion/cutover candidates because they do not each protect a unique invariant.

All executed tests passed, including 484 workspace library tests. That does **not** weaken these findings: the P0 rewrite-plus-append row is absent, the named backend gate omits 289 extracted-crate library tests, and the standalone formal model passes against obsolete codec vectors. The recommended sequence is correctness proof first, then over-detection boundaries, then deletion and state consolidation.`,
  },
  {
    id: "priority_distribution_intro",
    type: "markdown",
    layout: "full",
    body: `## The audit found one P0, thirteen P1s, and a concentrated P2 cleanup queue

The bar chart is a triage inventory, not an effort estimate: one finding is counted once at its highest recommended action level. The concentration in P1 means the next backend cycle should be framed as correctness and boundary repair before broad architectural cleanup. P2 remains substantial, but most of it becomes safer to delete after the P0/P1 predicates are executable tests.`,
  },
  {
    id: "priority_distribution_chart",
    type: "chart",
    layout: "full",
    chartId: "priority_distribution",
  },
  {
    id: "finding_register_intro",
    type: "markdown",
    layout: "full",
    body: `## The detailed register ranks risk before raw line reduction

“P1” does not mean every item is currently causing production incidents; it means the code admits a concrete silent-loss, invalid-proof, unrelated-blocker, or unbounded-cost sequence. Estimated reductions in the detailed sections intentionally overlap and should not be summed. The exact table is the lookup surface for scope, consequence, and disposition.`,
  },
  {
    id: "finding_register_table",
    type: "table",
    layout: "full",
    tableId: "finding_register",
  },
  {
    id: "accepted_prefix",
    type: "markdown",
    layout: "full",
    body: `## BACK-001: the accepted-prefix relation is inferred from count, not proved

**Concrete counterexample.** Let the accepted canonical chain be A, B(old), with accepted end count 2 and cumulative state H(A,B(old)). Replace B with equal-length B(new), preserve its ID/parent, and append C. The new full state is H(A,B(new),C), which does not extend the accepted state.

The relation classifier first checks exact current equality, then declares **LocalOutstanding** when cloud equals accepted and local count is greater, or **CloudOutstanding** symmetrically: ${code("crates/mallard-domain/src/conversation_tail_relation.rs", "179-195")}. It never checks the cumulative state at record 2. The reconciliation policy then maps those relations to Publish or MaterializeCloud: ${code("crates/mallard-domain/src/conversation_reconciliation.rs", "160-203")}.

**Why the other pins do not repair the decision.** A full Push reparse compares the current complete tail with the reviewed complete tail, which proves review-to-apply stability but not accepted-prefix continuity: ${code("src-tauri/src/project_sync/commands.rs", "8589-8599")}. A Pull refolds the complete downloaded cloud tail and compares it with the reviewed cloud tail, then installs it; it does not refold specifically at the old accepted end: ${code("src-tauri/src/project_sync/commands.rs", "10553-10579")}. If both review and apply see A,B(new),C, every check is internally consistent and the wrong merge decision survives.

**The cursor makes detection weaker, not safer.** A Codex checkpoint stores only a trailing 4 KiB probe: ${code("crates/mallard-domain/src/conversation_tail.rs", "340-363")}. Incremental refresh checks length plus that window, resumes the old accumulator, and folds appended bytes: ${code("crates/mallard-infrastructure/src/codex_history_graph.rs", "539-665")}. An equal-length edit before the probe can therefore produce a fictitious H(A,B(old),C). That first Push often fails late when full apply reconstruction finds the mismatch; a later full resolve exposes H(A,B(new),C), after which the count-only classifier can authorize it.

**Minimal durable repair.** Extend verified evidence with typed local/cloud “extends accepted” proofs, computed by retaining the cumulative state at the accepted boundary. Outstanding relations must require that proof; count alone is never sufficient. For the cursor, either store and re-prove a full accepted-range content digest or remove the delta refresh from reviewed reconciliation and use the already bounded full parser. Pull apply must re-prove the downloaded chain at the accepted boundary before journaling.

**Required tests.** Add local and cloud rewrite-plus-append rows to the domain matrix; add the earlier-than-4-KiB same-length mutation to the Codex graph suite; and add a Pull flow asserting that accepted/local A,B(old) versus cloud A,B(new),C is Divergent/blocked, never MaterializeCloud. Invalidate and reseed existing tail checkpoints in a clean schema cutover rather than adding compatibility interpretation.`,
  },
  {
    id: "state_loss",
    type: "markdown",
    layout: "full",
    body: `## BACK-002 and BACK-003: large aggregate reconstruction creates silent loss and half-transactions

### Relinking erases conversation checkpoints

${code("src-tauri/src/project_sync/commands.rs", "12522-12546")} constructs a fresh ProjectStorageLink with an empty conversation_checkpoints map. On an existing link it manually copies back created_at, recipe, scope bases, and project-content preferences, but not the checkpoints, then replaces the aggregate. The current regression test protects preferences but never seeds a checkpoint: ${code("src-tauri/src/project_sync/commands.rs", "17039-17073")}.

This is the classic failure mode of over-modeled mutation: changing one boolean (pinned) is implemented by rebuilding a large aggregate field-by-field. The smallest fix is also the safest—on the existing branch, mutate only existing.pinned and preserve the aggregate. Extend the test so every field except pinned is byte-for-byte equal.

### Remote identity reassignment is not old-or-new across crashes

The remote-connect flow removes project bindings, rewrites the project and every link's duplicated bundle ID, manually rolls bindings back only for one failure region, and finally creates a replacement binding: ${code("src-tauri/src/project_sync/commands.rs", "12562-12743")}. There is even a literal disabled policy branch in the flow. If the config mutation succeeds and final binding persistence fails, the project can retain the new identity without an active binding.

The repository already has the correct pattern for setup: prospective validation, a durable transaction saved before writes, dependency-ordered idempotent apply, and startup recovery (${code("src-tauri/src/project_sync/commands.rs", "13927-14296")}). Prefer deleting the special identity-mutation route and sending the user through that setup transaction. If reassignment remains a product capability, it needs the same journal/fixed-point model. Failure-injection tests must cover after binding removal, after config commit, before/inside binding creation, and two recovery passes.`,
  },
  {
    id: "path_and_codec",
    type: "markdown",
    layout: "full",
    body: `## BACK-004, BACK-005, and BACK-014: portability transforms are broader than their structural authority

### Codex path mapping mutates content, not only provider control fields

The mapper explicitly replaces project-root occurrences in **every string value and every object key**: ${code("crates/mallard-infrastructure/src/codex_path_map.rs", "1-13")} and ${code("crates/mallard-infrastructure/src/codex_path_map.rs", "308-402")}. Its boundary treats any character outside A–Z, a–z, 0–9, dot, underscore, and hyphen as a separator: ${code("crates/mallard-infrastructure/src/codex_path_map.rs", "126-135")}. Thus /a/proj in /a/proj+copy is eligible; prose, code, tool arguments, serialized command data, and keys are also eligible. Literal use of /__mallard_root__ blocks sync because no escaping exists: ${code("crates/mallard-infrastructure/src/codex_path_map.rs", "278-296")}.

The essential requirement is narrower: provider-owned path fields required to resume a session on another checkout must relocate. The long-term design should allowlist exact structural fields—session/turn context cwd and any provider field proven necessary—and preserve conversation/tool content byte-for-byte. That likely removes most of the 807-line generic mapper plus associated dual local/wire identity and collision machinery. This recommendation is policy-sensitive: before cutover, enumerate committed provider fixtures proving which non-cwd structural fields Codex actually consumes.

### Claude relocation proves only the first cwd

Claude analysis records the first structural cwd but does not reject later different values: ${code("crates/mallard-infrastructure/src/providers/claude/codec.rs", "72-92")}. Fingerprinting removes cwd, rewrite changes only values equal to the reviewed first source, and post-validation checks the first observed cwd plus semantic lineage (${code("crates/mallard-infrastructure/src/providers/claude/codec.rs", "254-271")} and ${code("crates/mallard-infrastructure/src/providers/claude/codec.rs", "327-369")}). A mixed-root transcript can therefore retain stale absolute paths and pass.

Require all structural cwd fields to agree during analysis, rewrite all, then re-prove all equal the destination. Separately, Claude memory discovery stores one cwd per bucket in a map, allowing a later owned session to overwrite a different earlier cwd: ${code("crates/mallard-infrastructure/src/providers/claude/discovery.rs", "72-106")}. Aggregate candidates per bucket and report ambiguity rather than guessing the last project subtree.`,
  },
  {
    id: "plugin_and_pet",
    type: "markdown",
    layout: "full",
    body: `## BACK-006 and BACK-007: detection is expensive where the actual validation boundary is permissive

### Plugin plans store executable vectors instead of semantic intent

NativePluginPlan persists provider, raw argv, marketplace argv arrays, and mode: ${code("src-tauri/src/storage_catalog/domain.rs", "2094-2106")}. Plan validation checks relationships and digests but not an external-command grammar (${code("src-tauri/src/storage_catalog/domain.rs", "2175-2223")} and ${code("src-tauri/src/storage_catalog/domain.rs", "2406-2463")}); apply executes the stored vectors through the runner: ${code("src-tauri/src/storage_catalog/apply.rs", "919-955")}.

Meanwhile skill inventory launches plugin discovery to infer ownership, Codex can launch two list subprocesses, and top-level provider documents default missing maps or malformed records toward empty/skipped states: ${code("src-tauri/src/storage_catalog/inventory.rs", "413-569")} and ${code("src-tauri/src/storage_catalog/adapters.rs", "348-490")}. Claude's raw-value parsing similarly turns missing/wrongly typed maps into empty inventory: ${code("src-tauri/src/storage_catalog/adapters.rs", "619-712")}. Apply then performs repeated pre/post discovery. The system pays for broad probing but a provider schema change can masquerade as “no plugins,” undermining the ownership exclusion the probing is supposed to prove.

Persist only semantic provider/mode/source intent. Require exact versioned external inventory shapes; unsupported schemas block that scope. Immediately before launch, reconstruct the exact no-shell argv in the adapter after a fresh authoritative inventory. Return the post-apply inventory from verification rather than discovering again. Standalone fixed skill roots should remain usable when plugin CLI discovery is unavailable, and a broad marketplace path must never hide an unrelated skill without exact ownership evidence.

### Pet image validation accepts headers, not images

PNG validation reads signature, IHDR marker, and dimensions and can accept roughly 24 bytes; WebP validation reads fixed offsets without proving RIFF/chunk lengths or image payload: ${code("src-tauri/src/storage_catalog/codex_pet.rs", "73-154")}. The E2E fixture institutionalizes a tiny header-like WebP: ${code("src-tauri/tests/support/catalog_flow.rs", "278-295")}.

Replace handwritten header inspection with a maintained decoder/container validator under existing encoded-size and decoded-allocation bounds. Test truncated-after-header data, invalid PNG chunk structure, invalid RIFF/chunk sizes, corrupt VP8/VP8L/VP8X payloads, valid format variants, exact dimensions, and decompression/allocation bombs.`,
  },
  {
    id: "uuid_allocator",
    type: "markdown",
    layout: "full",
    body: `## BACK-013: the fallback UUIDv7 is non-conforming and the preferred path is a subprocess protocol

The 634-line allocator first starts a disposable Codex app-server and speaks initialize/thread-start with timeouts, message/byte caps, child cleanup, and fallback behavior: ${code("crates/mallard-infrastructure/src/codex_thread_allocator.rs", "1-109")}. The local fallback fills all 128 bits randomly, then overwrites only the version and variant bits: ${code("crates/mallard-infrastructure/src/codex_thread_allocator.rs", "50-69")}. Validation checks text shape, the version nibble, and variant—but not the UUIDv7 timestamp field: ${code("crates/mallard-infrastructure/src/codex_thread_allocator.rs", "25-47")}.

[RFC 9562 §5.7](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7) defines UUIDv7's most significant 48 bits as Unix epoch milliseconds; random data belongs in the remaining 74 non-version/non-variant bits. The current comment that a random timestamp-less v7 is permitted is therefore incorrect.

Use a maintained local UUIDv7 generator—or UUIDv4 if Codex's accepted-ID contract permits it—and delete the app-server allocator unless a provider-owned allocation has a measured semantic benefit. Tests should decode a timestamp near the clock, prove version/variant, uniqueness, collision retry, and batch allocation with no process spawn.`,
  },
  {
    id: "remote_scale",
    type: "markdown",
    layout: "full",
    body: `## BACK-008 and BACK-009: per-call bounds mask namespace-wide and directory-wide work

Bundle discovery lists all objects below the repository namespace—up to one million keys—then filters tag objects and truncates to at most 500 repositories: ${code("crates/mallard-infrastructure/src/bundle_engine.rs", "1104-1173")}. Library discovery repeats the pattern and permits one million observed objects and 10,000 repositories: ${code("src-tauri/src/storage_catalog/resource_engine.rs", "23-28")} and ${code("src-tauri/src/storage_catalog/resource_engine.rs", "238-286")}. Payload and historical revision volume can therefore make repository discovery fail even when repository count is modest. A flat closed _tags or _index prefix per protocol makes pagination real and isolates discovery from payload scale. Publish the discovery record only in the successful head/tag publication path; use a clean key-schema cutover without fallback scans.

Catalog plans have the inverse problem locally. Each plan is bounded at 64 MiB and create-only, but list_resource_plans parses every JSON file without an aggregate entry/byte/live-plan bound: ${code("src-tauri/src/storage_catalog/persistence.rs", "25-31")} and ${code("src-tauri/src/storage_catalog/persistence.rs", "337-406")}. Storage reference checks, clear, Link, and Fork enumerate the directory again; expired or abandoned plans are not startup-pruned. Use one bounded active-plan slot keyed by registration/link and direction, atomically replace it, retire superseded IDs, prune expired plans under the lock, and impose small hard directory-entry/aggregate-byte limits.`,
  },
  {
    id: "over_detection",
    type: "markdown",
    layout: "full",
    body: `## BACK-010, BACK-011, and BACK-015: blockers are scoped to discovery, not to reviewed authority

**Any blocked conversation blocks the whole Push.** The code's own contract says a divergent/blocked thread holds back only itself, yet orchestration errors on the first blocked discovered candidate before resolving approved conversation IDs: ${code("src-tauri/src/project_sync/commands.rs", "8028-8031")} and ${code("src-tauri/src/project_sync/commands.rs", "9458-9473")}. A blocked unselected Codex thread can therefore stop a selected Claude thread or a project-file-only Push. Remove the inventory-wide blocker and let selected action capture/pin validation fail closed per action.

**Codex ownership readiness is global and duplicated.** Reviewed Push and Pull run a full Codex assignment/path audit before knowing whether any Codex action is selected, then run provider discovery again: ${code("src-tauri/src/project_sync/commands.rs", "9418-9458")} and ${code("src-tauri/src/project_sync/commands.rs", "9888-9955")}. The audit walks active and archived trees, parses Desktop state variants, and can pause all Push/Pull. Preserve canonical/no-follow/bounded ownership checks, but produce ownership evidence in the authoritative discovery snapshot and require it only for selected Codex actions. The explicit repair UI may remain global because the user asked for a global diagnostic.

**Project credential detection reuses provider-home traversal exclusions.** denied_file_name includes generic component names such as cache, plugins, marketplaces, and repos; the non-Git project scanner applies it to every relative path component and reports credential/private-key language: ${code("crates/mallard-infrastructure/src/provider_capture.rs", "1262-1268")} and ${code("crates/mallard-infrastructure/src/provider_capture.rs", "2178-2197")}. Split credential names from provider-home exclusions. The custom ignore parser (${code("crates/mallard-infrastructure/src/provider_capture.rs", "1506-1644")}) supports only a subset of Git semantics; use an established ignore library already compatible with no-follow traversal, or explicitly narrow the product claim.

**Read operations also mutate/guess authority.** ProjectSync and Catalog profile-list commands auto-create default profiles, changing revisions, and profile probing silently substitutes a child .codex/.claude home when present: ${code("src-tauri/src/project_sync/commands.rs", "2982-3186")}. Return ephemeral suggestions or bootstrap explicitly; surface detected children as choices rather than changing authority during a read.`,
  },
  {
    id: "state_architecture",
    type: "markdown",
    layout: "full",
    body: `## BACK-012, BACK-018, BACK-019, and BACK-020: boundaries multiply state without creating independent authority

### One physical provider home has two identity systems

The shared domain already defines ProviderProfile in machine project state: ${code("crates/mallard-domain/src/lib.rs", "1542-1607")}. Catalog separately defines LocalCatalogProfile, its own provider enum, repository, resolution, root construction, revisions, and reference/deletion logic: ${code("src-tauri/src/storage_catalog/domain.rs", "199-222")} and ${code("src-tauri/src/storage_catalog/domain.rs", "1230-1302")}. Extract one shared machine-local provider-profile document; ProjectSync bindings and Catalog scopes store the same ID, while each feature retains its own domain semantics and capability registry.

### Six Catalog documents are one consistency domain in practice

Profiles, resources, scopes, materializations, preservation, and operations use the same process/OS lock but load, revise, and atomically write independently: ${code("src-tauri/src/storage_catalog/persistence.rs", "19-40")} and ${code("src-tauri/src/storage_catalog/persistence.rs", "235-538")}. Unlink and clear manually sequence changes across several documents. After provider profiles are extracted, combine Catalog scopes, registrations/links, materialized provenance, and in-flight operations in one bounded CatalogStateDocument with one transact operation; keep large immutable plans separate behind the bounded active-plan index.

### Persistence defenses are duplicated with weaker variants

Catalog reimplements restricted directories, locks, ancestor checks, bounded JSON, and atomic writes: ${code("src-tauri/src/storage_catalog/persistence.rs", "778-941")}. The infrastructure implementation additionally rejects hard-linked locks and propagates permission/directory-fsync failures: ${code("crates/mallard-infrastructure/src/persistence.rs", "2413-2633")}. Share a small safe_fs module containing the strongest primitives, then keep feature-specific schema validation and quarantine policy local. The simplification is deduplication, not weakening no-follow, stable-read, permission, fsync, or corruption behavior.

### Per-conversation journals copy shared documents N times

Push and Pull build one journal operation per conversation, cloning evolving full config and binding before/after documents for each action: ${code("src-tauri/src/project_sync/commands.rs", "8832-8956")} and ${code("src-tauri/src/project_sync/commands.rs", "10768-10979")}. Preserve per-target staging/commit and individual preparation failure, but journal one batch with per-target subactions plus one config before/after and one bindings before/after. Commit shared documents once. Shrink failpoint tests only after production phases shrink.`,
  },
  {
    id: "surface_and_layers",
    type: "markdown",
    layout: "full",
    body: `## BACK-016 through BACK-018: the refactor introduced forwarding layers while production still lives in command controllers

ProjectSync commands.rs is 22,888 lines at the audit baseline, with roughly 15.5k production lines and 7.4k in-file tests. It registers 69 Tauri commands and includes provider discovery, planning, remote continuity, apply, recovery, profile management, setup transactions, path repair, and storage construction. ProjectSyncService imports commands and forwards back to command helpers, while commands instantiate the service; Catalog repeats the same direction: ${code("src-tauri/src/project_sync/service.rs", "1-271")} and ${code("src-tauri/src/storage_catalog/service.rs", "1-280")}.

This is not a cosmetic file-size complaint. Two entry paths have different preconditions. Candidate pins are stored “for the next review,” but every production service entry force-rebaselines and drops profile watchers/caches first: ${code("src-tauri/src/project_sync/codex_watch_service.rs", "766-859")} and ${code("src-tauri/src/project_sync/service.rs", "128-260")}. Tests that call internal repository helpers can demonstrate reuse the shipped service prevents. Delete only this candidate-pin cache; retain the observation/inventory cache used by Activity.

The Tauri surface is frozen to 94 identifiers by a test whose comment still says 95: ${code("src-tauri/src/sync_tests/registration_freeze.rs", "1-137")}. At least seven registered ProjectSync commands have no frontend wrapper, and a broader set of wrappers has no production consumer. Replace the historical exact-name freeze with parity/reachability: every frontend invoke must be registered; every registered application command must have an active consumer or a small explicit maintenance allowlist. Then remove dead handlers/DTOs in one cutover and use structured request objects instead of frozen too-many-argument signatures.

Finish the dependency direction: application services own request types and use cases; Tauri commands resolve repositories/app handles, call one service method, and emit progress/activity. Integration tests call the same service path. Delete temporary module re-export shims only after callsites move; do not merely split the giant file into arbitrary slices.`,
  },
  {
    id: "unused_and_repeated",
    type: "markdown",
    layout: "full",
    body: `## BACK-021 and BACK-022: several schemas and hot paths have no unique consumer

**Unused remote objects.** Bundle and Library publication each writes a commit object, but production has no commit reader; heads already carry revision, commit ID, manifest key/hash/size and immutable manifests preserve history: ${code("crates/mallard-infrastructure/src/bundle_engine.rs", "430-445")} and ${code("src-tauri/src/storage_catalog/domain.rs", "1136-1189")}. Remove both commit types/uploads/key constructors and bundle file_delta in one key-schema cutover. Managed orphan commit objects can be deleted by storage clear; do not add a compatibility read path.

**Impossible/speculative type axes.** Provider capability levels have four variants, but Codex and Claude always report the highest and production never branches on the lower levels: ${code("crates/mallard-infrastructure/src/providers/traits.rs", "20-50")}. Plugin source/resolution/requirement types encode variants planners cannot construct or validators reject. Codex pet format already exists as data but is multiplied into resource variants, kind tags, scope rows, filters, and tests. Remove capability state until a real provider differs; derive plugin resolution from optional portable source; model one CodexPet resource with format in stable identity.

**Repeated work.** Push destination continuity is probed twice; remote snapshot listing inspects every bundle—up to 10,000 manifests—to build a first page; apply-time closures reload config per conversation and use .ok(); capture can secret-scan, analyze, and summarize large bytes through separate passes; session detail can build a timeline and parse again for summary; plugin apply repeatedly discovers before/after verification. Replace each with one typed authoritative snapshot returned by the lower layer. A global transcript-analysis cache that can retain thousands of analyses of up to 100 MiB conversations should be removed or byte-bounded; its sole production purpose is a legacy stored-subagent compatibility path that conflicts with the repository's clean-cutover policy.`,
  },
  {
    id: "verification_intro",
    type: "markdown",
    layout: "full",
    body: `## Green tests expose a gate-design problem, not proof that the findings are unreachable

The verification run is useful: it shows the repository is internally stable enough to refactor and that the findings are not artifacts of a generally broken checkout. It also demonstrates exactly where coverage claims are too broad. The workspace suite passes while missing the accepted-prefix counterexample; the named backend script skips most extracted-crate tests; and the formal model passes its own obsolete hash vocabulary.`,
  },
  {
    id: "verification_table",
    type: "table",
    layout: "full",
    tableId: "verification",
  },
  {
    id: "verification_detail",
    type: "markdown",
    layout: "full",
    body: `## BACK-023: align the executable gate with the extracted architecture

package.json defines test:backend-integration as the Tauri library tests plus the Catalog E2E. It omits the 44 domain, 219 infrastructure, and 26 application library tests—289 tests around the extracted safety kernel. Make cargo test --workspace --lib the backend unit gate, then run the storage E2E. Keep the full end-to-end tests; their size reflects real cross-boundary behavior and they caught useful privacy/CAS invariants.

The formal script pins empty-whole tail hash 85ca… at ${code("scripts/verify_portable_tail_conflict_model.py", "861-899")}, while current Rust pins 267171… at ${code("crates/mallard-domain/src/tests.rs", "1446-1499")}. Both pass independently. Generate vectors from one reviewed external fixture or have one gate compare the model's vectors directly with the Rust codec, and wire the script into CI/package integration. Extend the model vocabulary with explicit accepted-boundary extension evidence; otherwise it will continue proving the same invalid count-implies-prefix assumption as production.`,
  },
  {
    id: "prior_intro",
    type: "markdown",
    layout: "full",
    body: `## The previous audit was partly acted on, but several “fixed-looking” abstractions left narrower gaps

Prior documents were used only as a comparison baseline; every status below was rechecked in current code. Exact provider metadata parsing, Activity read sharing, and single-resource Catalog filtering materially improved the system. The new P0 was introduced or exposed by the newer tail-evidence/cursor architecture: centralizing a classifier reduced state explosion but did not add the one extension proof the classifier needs.`,
  },
  {
    id: "prior_table",
    type: "table",
    layout: "full",
    tableId: "prior_status",
  },
  {
    id: "scope",
    type: "markdown",
    layout: "full",
    body: `## Scope and definitions

**Evidence baseline.** Committed HEAD ${commit} on 2026-08-18. The workspace had concurrent uncommitted frontend and backend edits; they were preserved and excluded from finding evidence. Line references target the committed snapshot. Backend scope includes Rust domain/application/infrastructure crates, the Tauri command boundary, provider capture/codecs, local persistence, storage protocols, Catalog, Activity, CLI seams, backend tests, and backend package gates. Frontend behavior was consulted only to determine whether a command has a current consumer.

**Over-engineered** means a state object, abstraction, cache, transaction phase, schema, or command exists without protecting a distinct invariant or serving a current capability. Large code is not automatically over-engineered.

**Over-detection** means a broad scan or heuristic blocks content/actions outside the user's reviewed authority, or ordinary content is treated as credential/path/provider evidence without an exact structural basis.

**False safety** means a validation layer is present but proves a weaker predicate than the decision requires—for example count instead of prefix, image dimensions instead of a complete image, or empty inventory instead of a supported external schema.

**Priority.** P0 can authorize silent history corruption/adoption. P1 is a concrete silent state-loss, crash-consistency, invalid external-boundary, or normal-operation availability/scaling issue. P2 is bounded correctness/performance/maintainability debt. P3 requires an explicit product-support decision. Confidence distinguishes code evidence from the policy choice implied by a recommendation.`,
  },
  {
    id: "methodology",
    type: "markdown",
    layout: "full",
    body: `## Methodology

The audit traced supported product flows from Tauri registration through service wrappers, domain decisions, provider adapters, persistence, transport, apply/recovery, and tests. For safety checks, the question was not “is there a pin?” but “does this pin re-acquire independent live evidence for the exact predicate used to authorize mutation?” For scale, per-file/per-page bounds were followed outward to aggregate namespace and directory work. For over-detection, blocker scope was compared with the reviewed selection and provider/resource authority. Reachability was checked from registered commands through frontend wrappers/callsites. Prior-audit claims were classified fixed, partial, open, or product-dependent only after current-code reinspection.

No quantitative chart is included. The few counts—lines, commands, tests, object/plan bounds—are inventory context, not a meaningful time series or comparable magnitude model; a chart would imply precision and comparability that the static audit does not support. Exact lookup tables are the more honest evidence format.`,
  },
  {
    id: "edge_intro",
    type: "markdown",
    layout: "full",
    body: `## The edge-case backlog should be executable before the architecture is simplified

These cases are deliberately small and adversarial. Each isolates the predicate a fix must establish, which makes them better guards than another broad end-to-end scenario. Add the P0/P1 cases first; retain current crash/CAS/privacy matrices while reducing production phases, then shrink only obsolete rows.`,
  },
  {
    id: "edge_table",
    type: "table",
    layout: "full",
    tableId: "edge_cases",
  },
  {
    id: "retain",
    type: "markdown",
    layout: "full",
    body: `## Complexity that should remain

The following code may look elaborate, but it protects independent mutable resources or real transport/filesystem failure modes:

- **Immutable publication plus mutable-head CAS.** Uploading content-addressed objects before head CAS can leave unreferenced objects; that is safer than rollback choreography. Garbage collection should be a separate measured feature.
- **Verified bodies and the ObjectStore abstraction.** Local folder and S3/R2 genuinely share a closed object-key protocol; disk-backed verified bodies prevent unbounded memory use.
- **S3 ambiguity, pagination, and lexical-neighbor handling.** Ambiguous network outcomes around immutable put, CAS, and delete need reconciliation, not optimistic simplification.
- **No-follow/canonical/special-file defenses and bounded stable reads.** Preserve path containment, symlink/hard-link/special-file rejection, changed-during-read checks, permissions, fsync, and byte/count limits. Consolidate implementations, not invariants.
- **Review/apply revalidation.** Expiry, identity, binding/profile revisions, head coordinates, target digests/modes, checkpoint generation, warning acknowledgements, and exact deletion rules protect different TOCTOU surfaces. Remove only copied/tautological fields that are not independently re-acquired.
- **Active filesystem journals, staging, backups, and no-clobber activation.** These protect user data under crash and partial failure. Simplify redundant permanent witnesses/terminal history, not the in-flight transaction.
- **Setup transaction and startup recovery.** It is proportionate because config and machine bindings are separate current documents. Reuse it or consolidate the documents; do not replace it with manual rollback.
- **Warning-only secret/executable detection.** Best-effort warnings plus explicit acknowledgement match the product contract. Private-key hard blocks remain justified. The problem is applying provider-home directory exclusions as project credential evidence.
- **Provider registry and two storage transports.** Codex/Claude and local/S3 are real implementations. Remove the unused capability ladder, not the modular boundary.
- **Codex graph/fork validation and recovery failpoint tests.** Keep while paginated/forked sessions are supported. Tests should contract only after the production state machine contracts.
- **Managed R2 provisioning.** Its bounded non-secret journal and temporary-token lifecycle are proportionate if automatic provisioning is a product requirement. Removing it is a product choice, not a local cleanup.`,
  },
  {
    id: "roadmap",
    type: "markdown",
    layout: "full",
    body: `## Recommended cut sequence

### Phase 0 — close false proofs and silent state loss

1. Add accepted-boundary extension evidence; remove or harden the 4 KiB cursor; re-prove Pull at the accepted end; invalidate checkpoints.
2. Change existing-link update to mutate pinned only and add preservation coverage.
3. Delete the special remote-identity mutation route or put it under the durable setup transaction with failpoints.
4. Reject mixed Claude cwd and ambiguous memory buckets; replace the UUID allocator with a conforming local generator.
5. Make plugin plans semantic with strict external schemas; replace pet header checks with real bounded validation.

### Phase 1 — scope detection to reviewed authority

1. Apply conversation blockers and Codex path ownership only to selected actions.
2. Narrow Codex path transformation to provider structural fields.
3. Split project credential deny lists from provider-home exclusions and adopt complete ignore semantics.
4. Make profile listing read-only and path-child detection explicit.
5. Add flat remote discovery indexes, lazy bundle inspection, and bounded active plan slots.

### Phase 2 — delete dead paths before moving code

1. Replace the 94-name freeze with registration/reachability parity; remove dormant commands and the legacy stored-subagent content-sniff path.
2. Delete candidate-pin cache state, unused remote commit objects, unused capability levels, impossible plugin requirement variants, and redundant pet kind variants.
3. Move use cases into application services and make Tauri commands thin. Delete temporary re-export shims after consumers move.
4. Unify provider profiles and strongest safe_fs primitives in clean machine-local/schema cutovers.

### Phase 3 — reduce transaction and hot-path state

1. Consolidate Catalog into one atomic state document plus bounded immutable plans.
2. Batch conversation shared-document commits under one journal while retaining per-target stages/failures.
3. Fuse parse outputs into typed snapshots for capture, Activity/detail, plugin apply, continuity, and config/checkpoint validation.
4. Decide explicitly whether Activity is queryable history (SQLite) or human-readable JSONL, whether Codex 0.147 remains supported, and whether automatic R2 provisioning remains in scope.

**Definition of done.** Each deletion must remove its compatibility path, state/DTO fields, registration, tests that preserve obsolete behavior, and unused dependency. Each safety repair must add the smallest counterexample test at the protected helper plus one cross-boundary flow test.`,
  },
  {
    id: "limitations",
    type: "markdown",
    layout: "full",
    body: `## Limitations, uncertainty, and robustness checks

This is a static, repository-grounded audit, not a production incident analysis or benchmark. It did not touch real provider homes, Mallard state, credentials, local object repositories, S3/R2, or provider CLIs. Remote-scan, heap, subprocess, and latency findings are derived from explicit bounds/call graphs; measure representative repositories before promising a performance number. LOC reduction estimates from the underlying audit overlap heavily and are intentionally omitted from the register rather than summed.

The committed baseline and test workspace differ because the checkout was modified concurrently. Findings use committed ${commit}; verification commands passed against the current dirty workspace. The relevant concurrent backend diffs were small test/style changes, but a clean-tree CI rerun should accompany fixes.

Some recommendations are product-sensitive. Narrowing generic Codex path mapping requires provider fixtures that prove which structural fields are authoritative. Deleting frozen 0.147 handling requires an explicit current-format support policy. Replacing Activity JSONL depends on whether “open logs folder” is a durable human-readable contract. Managed R2 removal would trade substantial code for setup/security UX and should not be smuggled into a refactor.

The strongest robustness check was negative-case construction: when a check looked duplicated, the audit sought a state in which the two checks observe different mutable resources. That is why CAS, target digests, expiry, no-follow validation, and active journals are retained, while copied plan fields, count-only relation logic, header-only image checks, and read-before-selection blockers are challenged.`,
  },
  {
    id: "further_questions",
    type: "markdown",
    layout: "full",
    body: `## Further questions that change the optimal cut

1. Is byte-for-byte preservation of user/assistant/tool content a hard conversation contract, with only provider-owned structural fields relocatable?
2. Are pre-current Codex 0.147/paginated transcripts expected to remain resumable, or may they become stored-only after a clean cutover?
3. Is changing an already registered project's remote bundle identity still a supported user action, or can setup be the only identity-establishment path?
4. Is human-readable JSONL Activity history a product contract, or is queryable local history more important?
5. May machine-local schema changes reset/re-register local state, consistent with the repository's no-compatibility policy?
6. Does provider thread ID generation require a Codex-owned allocator for any reason beyond syntactic UUID acceptance?
7. Which provider plugin schema versions are intentionally supported by committed fixtures? Unsupported versions should block rather than appear empty.
8. What representative upper bounds should drive load tests: conversations per project, transcript bytes, Catalog links/plans, remote bundles, and remote payload objects?`,
  },
];

const artifact = {
  surface: "report",
  manifest: {
    version: 1,
    surface: "report",
    title: "Mallard Backend Complexity, Over-Detection, and Edge-Case Audit",
    description: `Backend-only technical audit anchored to commit ${commit}, with prioritized correctness defects, over-detection boundaries, simplification candidates, edge-case tests, and a staged cut sequence.`,
    generatedAt,
    blocks,
    tables: [
      {
        id: "finding_register",
        title: "Prioritized backend findings",
        subtitle: "Ranked by integrity, crash consistency, unrelated blocking, and pathological cost—not by raw line count",
        showDescription: true,
        dataset: "findings",
        defaultSort: { field: "rank", direction: "asc" },
        density: "spacious",
        layout: "full",
        source: tableSource({
          id: "finding_register_source",
          label: `Prioritized findings derived from backend source at ${commit}`,
          href: `${repositoryUrl}/tree/${commit}`,
          rows: findings,
          columns: ["rank", "id", "priority", "confidence", "class", "finding", "consequence", "disposition"],
          orderBy: '"rank" ASC',
          description: "Exact bounded report snapshot of findings produced by read-only code inspection and counterexample tracing.",
        }),
        columns: [
          { field: "rank", label: "Rank", type: "number" },
          { field: "id", label: "ID", type: "text" },
          { field: "priority", label: "Priority", type: "text" },
          { field: "confidence", label: "Confidence", type: "text" },
          { field: "class", label: "Class", type: "text" },
          { field: "finding", label: "Finding", type: "text" },
          { field: "consequence", label: "Consequence", type: "text" },
          { field: "disposition", label: "Recommended disposition", type: "text" },
        ],
      },
      {
        id: "verification",
        title: "Verification ledger",
        subtitle: "Commands executed during the audit and the boundary each result does not cover",
        showDescription: true,
        dataset: "verification",
        defaultSort: { field: "check", direction: "asc" },
        density: "spacious",
        layout: "full",
        source: tableSource({
          id: "verification_table_source",
          label: "Audit verification command ledger",
          href: `${repositoryUrl}/blob/${commit}/package.json`,
          rows: verification,
          columns: ["check", "result", "coverage", "caveat"],
          orderBy: '"check" ASC',
          description: "Exact bounded snapshot of commands executed, observed results, covered scope, and known caveats.",
        }),
        columns: [
          { field: "check", label: "Check", type: "text" },
          { field: "result", label: "Result", type: "text" },
          { field: "coverage", label: "Coverage", type: "text" },
          { field: "caveat", label: "Caveat", type: "text" },
        ],
      },
      {
        id: "prior_status",
        title: "Status of the 2026-08-17 backend findings",
        subtitle: "Current-code revalidation; prior documents were comparison inputs, not implementation authority",
        showDescription: true,
        dataset: "prior_status",
        defaultSort: { field: "status", direction: "asc" },
        density: "spacious",
        layout: "full",
        source: tableSource({
          id: "prior_status_source",
          label: "Revalidated 2026-08-17 backend audit status",
          href: `${repositoryUrl}/tree/${commit}/docs`,
          rows: priorStatus,
          columns: ["item", "status", "evidence"],
          orderBy: '"status" ASC, "item" ASC',
          description: "Exact bounded status snapshot after rechecking prior audit claims against current committed implementation.",
        }),
        columns: [
          { field: "item", label: "Prior issue", type: "text" },
          { field: "status", label: "Status", type: "text" },
          { field: "evidence", label: "Current evidence", type: "text" },
        ],
      },
      {
        id: "edge_cases",
        title: "Executable edge-case backlog",
        subtitle: "Small counterexamples that define the required behavior before broad refactoring",
        showDescription: true,
        dataset: "edge_cases",
        defaultSort: { field: "id", direction: "asc" },
        density: "spacious",
        layout: "full",
        source: tableSource({
          id: "edge_case_source",
          label: `Executable edge cases derived from backend source at ${commit}`,
          href: `${repositoryUrl}/tree/${commit}`,
          rows: edgeCases,
          columns: ["id", "priority", "area", "trigger", "current", "required"],
          orderBy: '"id" ASC',
          description: "Exact bounded backlog of minimal counterexamples and the required safe outcome for each.",
        }),
        columns: [
          { field: "id", label: "Case", type: "text" },
          { field: "priority", label: "Priority", type: "text" },
          { field: "area", label: "Area", type: "text" },
          { field: "trigger", label: "Trigger", type: "text" },
          { field: "current", label: "Current risk/behavior", type: "text" },
          { field: "required", label: "Required outcome", type: "text" },
        ],
      },
    ],
    charts: [
      {
        id: "priority_distribution",
        title: "Findings by priority",
        subtitle: "25 backend findings, counted once at the highest recommended action level",
        showDescription: true,
        intent: "comparison",
        question: "How is the backend action queue distributed across P0 through P3?",
        rationale: "A single-series bar chart makes four discrete priority counts easier to compare than prose while preserving exact values in the source table.",
        type: "bar",
        dataset: "priority_counts",
        source: {
          id: "priority_counts_source",
          label: `Priority classification derived from the ${commit} audit register`,
          href: `${repositoryUrl}/tree/${commit}`,
          query: {
            engine: "SQLite-compatible VALUES query",
            language: "sql",
            description: "Reproducible count of the 25 reviewed finding rows by their highest assigned priority; classification is the static code-audit judgment, not a production metric.",
            sql: "WITH findings(priority) AS (VALUES ('P0'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P1'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P2'), ('P3')) SELECT priority, COUNT(*) AS count FROM findings GROUP BY priority ORDER BY CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 3 END",
            executed_at: generatedAt,
            filters: ["Each finding counted once at its highest assigned priority", "Committed backend baseline cc6478e"],
            metric_definitions: ["count = number of reviewed findings assigned to the priority"],
          },
        },
        encodings: {
          x: { field: "priority", type: "ordinal", label: "Priority" },
          y: { field: "count", type: "quantitative", aggregate: "none", format: "number", label: "Findings" },
          tooltip: [
            { field: "priority", type: "ordinal", label: "Priority" },
            { field: "count", type: "quantitative", format: "number", label: "Findings" },
            { field: "meaning", type: "text", label: "Meaning" },
          ],
        },
        valueFormat: "number",
        layout: "full",
        maxRows: 4,
      },
    ],
    sources,
  },
  snapshot: {
    version: 1,
    generatedAt,
    status: "ready",
    datasets: {
      findings,
      priority_counts: priorityCounts,
      verification,
      prior_status: priorStatus,
      edge_cases: edgeCases,
    },
    accessIssues: [],
  },
  sources,
};

const outputPath = join(dirname(fileURLToPath(import.meta.url)), "artifact.json");
writeFileSync(outputPath, `${JSON.stringify(artifact, null, 2)}\n`, "utf8");
console.log(outputPath);

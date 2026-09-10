import { CANARIES, sha256 } from "./scenario.mjs";

export function safeFailure(error, stage) {
  const message = String(error.message ?? "unknown_failure");
  const result = { stage, code: /^[a-zA-Z0-9_]{1,100}$/u.test(message) ? message : "assertion_failed" };
  const first = message.split("\n", 1)[0];
  if (/^[A-Za-z0-9 _.,:;=()-]{1,200}$/u.test(first) && Object.values(CANARIES).every((s) => !first.includes(s))) result.assertion = first;
  const location = error.stack?.match(/scripts\/([^():\n]+:\d+:\d+)/u)?.[1];
  if (location && /^[a-zA-Z0-9_./:-]+$/u.test(location)) result.location = `scripts/${location}`;
  if (error.privateDetail) {
    const text = String(error.privateDetail);
    result.detail_digest = sha256(text);
    result.fixture_labels = [...new Set(text.match(/\b[CLK]\d{3}\b/gu) ?? [])].slice(0, 132);
    const code = text.match(/code=\d+,index=\d+,observed=\d+,total=\d+/u)?.[0];
    if (code) result.mock_mismatch = code;
  }
  return result;
}

export function summary(report) {
  const lines = ["# Mixed aimock result", "", `Status: ${report.status}.`, "", `Profile: ${report.profile ?? "unavailable"}.`, "",
    `Fixture: ${report.fixture ?? "unavailable"}.`, ""];
  if (report.sessions) lines.push(`Source: ${report.sessions.codex} Codex and ${report.sessions.claude} Claude sessions.`, "",
    `Source turns: ${report.source_turns ?? "incomplete"}. Final conversations: ${report.final_sessions?.total ?? "incomplete"}.`, "");
  if (report.graph) lines.push(`Codex graph: ${report.graph.edges} edges. Maximum depth: ${report.graph.max_depth}.`, "");
  if (report.failure) lines.push(`Failed step: ${report.failure.stage}. Code: ${report.failure.code}.`, "");
  const benchmark = report.benchmark;
  if (benchmark?.candidate?.operations?.length) {
    lines.push(`Benchmark: ${benchmark.comparison?.status ?? "incomplete"}.`, "",
      "| Operation | Samples | Median ms | p95 ms | Maximum RSS MiB |", "| --- | ---: | ---: | ---: | ---: |");
    for (const operation of benchmark.candidate.operations) {
      lines.push(`| ${operation.name} | ${operation.samples.length} | ${operation.median_ms.toFixed(3)} | ${operation.p95_ms.toFixed(3)} | ${(Math.max(...operation.samples.map((s) => s.peak_rss_bytes)) / 1048576).toFixed(1)} |`);
    }
    lines.push("");
    for (const item of benchmark.comparison?.comparisons?.filter((c) => c.regression) ?? []) {
      lines.push(`- Limit exceeded: ${item.operation}, ${item.metric}.`);
    }
  }
  lines.push("", `Whole job: ${(report.elapsed_ms / 1000).toFixed(1)} seconds.`, "");
  return lines.join("\n");
}

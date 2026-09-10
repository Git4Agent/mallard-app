import assert from "node:assert/strict";
import { LineProcess, environment } from "./process.mjs";

export class ServiceWorker {
  constructor(binary, root, replica, measurements, workers = 4) {
    this.replica = replica; this.workers = workers; this.measurements = measurements;
    this.process = new LineProcess(binary, ["mixed_aimock_worker", "--ignored", "--exact", "--nocapture", "--test-threads=1"],
      { cwd: replica.base, prefix: "MIXED_REPLY ", env: environment(replica.home, {
        MALLARD_RUN_MIXED_AIMOCK_E2E: "1", MALLARD_MIXED_ROOT: root,
      }) });
  }
  async call(operation, args = {}, allowError = false) {
    const id = ++this.process.counter;
    this.process.send({ id, operation, args });
    const result = await this.process.next();
    assert.equal(result.id, id, "worker response identity");
    if (!result.ok && !allowError) {
      const error = new Error(`service_${operation}_failed`);
      error.privateDetail = result.error; throw error;
    }
    if (result.ok && operation !== "open") this.measurements.push({
      operation, replica: this.replica.name, ms: result.ms, metrics: result.metrics,
      usage: result.usage, cpu_ms: result.usage?.cpu_ms - result.cpu_before?.cpu_ms,
    });
    return result;
  }
  async open(bundle) { return (await this.call("open", { workers: this.workers, replica: this.replica.name, project: this.replica.project, ...(bundle ? { bundle } : {}) })).value.project; }
  async compare() { return (await this.call("compare")).value; }
  async push(selection) {
    const review = (await this.call("push_review")).value;
    assert.equal(review.thread_comparison_error ?? null, null, "Push comparison error");
    const rows = review.thread_comparison.entries;
    const selected = selection ? rows.filter((row) => selection.has(row.provider_thread_id)) : rows;
    const result = (await this.call("push_apply", { resource_ids: selected.map((row) => row.resource_id) })).value.result;
    assert.equal(result.success, true, "Push failed");
    return result;
  }
  async pull() {
    const plan = (await this.call("pull_review")).value;
    const result = (await this.call("pull_apply")).value.result;
    assert.equal(result.success, true, "Pull failed");
    assert.equal(result.failed_actions.length, 0, "Pull action failure");
    return { plan, result };
  }
  close() { return this.process.close(); }
  kill() { this.process.kill(); }
}

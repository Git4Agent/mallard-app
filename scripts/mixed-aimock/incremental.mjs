import { Ledger } from "./roundtrip.mjs";
import { ServiceWorker } from "./service.mjs";

// Prepare native mutations outside every measured operation. All samples replay
// these fixed snapshots; no model call runs inside a benchmark sample.
export async function incrementalCheckpoints({ binary, root, replicas, checkpoints, native, nodes, clouds, bundle, workers, incremental }) {
  const definitions = Object.entries(incremental).map(([name, labels]) => ({ name, labels }));
  for (const definition of definitions) {
    await checkpoints.restore("accepted");
    const active = {};
    const open = async () => {
      for (const name of ["a", "b"]) { active[name] = new ServiceWorker(binary, root, replicas[name], [], workers); await active[name].open(name === "a" ? undefined : bundle); }
    };
    const close = async () => { for (const worker of Object.values(active)) await worker.close(); };
    await open();
    const ledger = new Ledger(native, nodes, replicas, active, clouds);
    try {
      await ledger.append("a", definition.labels, `BENCH_${definition.name.toUpperCase()}`);
      await close(); await checkpoints.save(`${definition.name}-local`); await open();
      await ledger.publish("a", definition.labels);
      await close(); await checkpoints.save(`${definition.name}-cloud`);
    } finally { for (const worker of Object.values(active)) worker.kill(); }
  }
}

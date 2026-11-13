"""Probe an installed Pi CLI with synthetic sessions; no SDK download or model call."""

import argparse
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import tempfile
import time


def check(condition, code):
    if not condition:
        raise RuntimeError(code)


class Rpc:
    def __init__(self, executable, session, project, env):
        self.process = subprocess.Popen(
            [executable, "--mode", "rpc", "--session", str(session),
             "--provider", "poc", "--model", "fixture", "--no-tools", "--no-mcp",
             "--no-extensions", "--no-skills", "--no-prompt-templates",
             "--no-themes", "--no-context-files", "--no-approve"],
            cwd=project, env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ, "stdout")
        self.selector.register(self.process.stderr, selectors.EVENT_READ, "stderr")
        self.buffer = b""
        self.bytes_read = 0
        self.sequence = 0

    def request(self, kind, **fields):
        self.sequence += 1
        request_id = "probe-" + str(self.sequence)
        command = dict(fields, type=kind, id=request_id)
        self.process.stdin.write((json.dumps(command) + "\n").encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            while b"\n" in self.buffer:
                line, self.buffer = self.buffer.split(b"\n", 1)
                if not line.strip():
                    continue
                response = json.loads(line)
                if response.get("type") == "response" and response.get("id") == request_id:
                    return response
            for key, _ in self.selector.select(timeout=0.25):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    self.selector.unregister(key.fileobj)
                    continue
                self.bytes_read += len(chunk)
                check(self.bytes_read <= 8 * 1024 * 1024, "rpc_output_bound")
                if key.data == "stdout":
                    self.buffer += chunk
            check(self.process.poll() is None or b"\n" in self.buffer, "rpc_exited_before_response")
        raise RuntimeError("rpc_response_timeout")

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.selector.close()
        self.process.stdout.close()
        self.process.stderr.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pi", default="pi", help="Installed Pi executable")
    args = parser.parse_args()
    executable = shutil.which(args.pi)
    check(executable is not None, "pi_not_installed")
    with tempfile.TemporaryDirectory(prefix="mallard-pi-rpc-poc-") as temporary:
        root = Path(temporary).resolve()
        project, agent, home = (root / name for name in ["project", "agent", "home"])
        for directory in [project, agent, home]:
            directory.mkdir()
        env = {
            "PATH": os.environ["PATH"], "HOME": str(home), "USERPROFILE": str(home),
            "XDG_CONFIG_HOME": str(home / "config"), "TMPDIR": str(root),
            "PI_CODING_AGENT_DIR": str(agent), "PI_OFFLINE": "1",
            "PI_SKIP_VERSION_CHECK": "1", "PI_TELEMETRY": "0",
        }
        version = subprocess.run([executable, "--version"], cwd=project, env=env,
                                 capture_output=True, text=True, timeout=20, check=True).stdout.strip()
        check(version == "1.5.2", "unverified_pi_version")
        # Local, unreachable metadata-only provider; no prompt/compact RPC is sent.
        (agent / "models.json").write_text(json.dumps({"providers": {"poc": {
            "baseUrl": "http://127.0.0.1:1/v1", "api": "openai-completions",
            "apiKey": "synthetic-poc-key", "models": [{
                "id": "fixture", "name": "Synthetic fixture", "reasoning": False,
                "input": ["text"], "contextWindow": 32768, "maxTokens": 1024,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            }],
        }}}))
        stamp = "2026-11-12T00:00:00Z"
        records = [{"type": "session", "version": 3, "id": "pi-rpc-poc",
                    "timestamp": stamp, "cwd": str(project)}]

        def entry(kind, identity, parent, **fields):
            records.append(dict(fields, type=kind, id=identity, parentId=parent, timestamp=stamp))

        def message(identity, parent, role, text):
            body = {"role": role, "content": text, "timestamp": 1794441600000}
            if role == "assistant":
                body.update(content=[{"type": "text", "text": text}], api="openai-completions",
                            provider="poc", model="fixture", stopReason="stop",
                            usage={"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                                   "totalTokens": 2, "cost": {"input": 0, "output": 0,
                                   "cacheRead": 0, "cacheWrite": 0, "total": 0}})
            entry("message", identity, parent, message=body)

        entry("model_change", "model001", None, provider="poc", modelId="fixture")
        entry("thinking_level_change", "think001", "model001", thinkingLevel="off")
        message("user0001", "think001", "user", "Original synthetic prompt.")
        message("answer01", "user0001", "assistant", "Abandoned branch answer.")
        message("user0002", "user0001", "user", "Active branch prompt.")
        message("answer02", "user0002", "assistant", "Active branch answer.")
        entry("compaction", "compact1", "answer02", summary="Synthetic summary.",
              firstKeptEntryId="user0002", tokensBefore=100)
        entry("custom", "custom01", "compact1", customType="poc", data={"count": 1})
        entry("session_info", "info0001", "custom01", name="Synthetic RPC storage probe")
        parent_file = root / "parent.jsonl"
        original = ("\n".join(json.dumps(record) for record in records) + "\n").encode()
        parent_file.write_bytes(original)
        rpc = Rpc(executable, parent_file, project, env)
        try:
            all_entries = rpc.request("get_entries")
            check(all_entries["success"] and len(all_entries["data"]["entries"]) == 9, "rpc_all_entries")
            tail = rpc.request("get_entries", since="answer01")
            check(tail["success"] and len(tail["data"]["entries"]) == 5, "rpc_incremental_since")
            check("nextCursor" not in tail["data"], "rpc_no_page_cursor")
            context = rpc.request("get_messages")
            check(context["success"] and len(context["data"]["messages"]) == 3, "rpc_compacted_context")
            check(not rpc.request("get_entries", since="missing-entry")["success"], "rpc_unknown_cursor_refused")
            check(rpc.request("clone")["success"], "rpc_clone")
            state = rpc.request("get_state")
            clone = Path(state["data"]["sessionFile"]).resolve()
            check(root in clone.parents and clone != parent_file, "rpc_clone_is_temporary")
            header = json.loads(clone.read_text().splitlines()[0])
            check(header["parentSession"] == str(parent_file), "rpc_clone_parent_reference")
        finally:
            rpc.close()
        check(parent_file.read_bytes() == original, "rpc_parent_unchanged")
        parent_file.rename(root / "unavailable-parent")
        rpc = Rpc(executable, clone, project, env)
        try:
            resumed = rpc.request("get_messages")
            check(resumed["success"] and resumed["data"] == context["data"], "rpc_clone_reopens_without_parent")
        finally:
            rpc.close()
        print(json.dumps({"pi": version, "installedCli": True, "rawEntries": 9,
                          "entriesAfterCursor": 5, "compactedMessages": 3,
                          "cloneReopenedWithoutParent": True, "modelRequests": 0}))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Never print transcript data, environment values, or local paths.
        print("Pi RPC POC failed: " + (str(error) if type(error) is RuntimeError else type(error).__name__))
        raise SystemExit(1)

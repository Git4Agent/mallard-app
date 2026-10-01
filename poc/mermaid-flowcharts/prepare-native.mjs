// Build and prepare the complete desktop application with isolated provider homes.
import { execFileSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
if (process.platform !== "darwin") throw new Error("The native UI fixture requires macOS");
const appRoot = fileURLToPath(new URL("../../", import.meta.url));
const fixtureRoot = await mkdtemp("/private/tmp/mallard-mermaid-qa-");
execFileSync("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "mermaid_native_ui_fixture", "--", "--ignored"], {
  cwd: appRoot, env: { ...process.env, MALLARD_MERMAID_QA_ROOT: fixtureRoot }, stdio: "inherit",
});
execFileSync("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--features", "tauri/custom-protocol", "--bin", "mallard"], { cwd: appRoot, stdio: "inherit" });
const bundle = resolve(fixtureRoot, "Isolated Mallard Session Diagrams QA.app");
await mkdir(resolve(bundle, "Contents/MacOS"), { recursive: true });
await copyFile(resolve(appRoot, "target/debug/mallard"), resolve(bundle, "Contents/MacOS/mallard"));
const launcher = `#!/usr/bin/python3
import os, json
from pathlib import Path
root = Path(${JSON.stringify(fixtureRoot)})
environment = dict(os.environ)
environment.update({"HOME": str(root / "home"), "CODEX_HOME": str(root / "codex"), "CLAUDE_CONFIG_DIR": str(root / "claude")})
(root / "launch-proof.json").write_text(json.dumps({key + "_is_fixture": environment[key] == str(root / value) for key, value in [("HOME", "home"), ("CODEX_HOME", "codex"), ("CLAUDE_CONFIG_DIR", "claude")]}))
binary = Path(__file__).with_name("mallard")
os.execve(str(binary), [str(binary)], environment)
`;
await writeFile(resolve(bundle, "Contents/MacOS/qa-launcher"), launcher, { mode: 0o755 });
await writeFile(resolve(bundle, "Contents/Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>qa-launcher</string>
<key>CFBundleName</key><string>Isolated Mallard Session Diagrams QA</string>
<key>CFBundleDisplayName</key><string>Isolated Mallard Session Diagrams QA</string>
<key>CFBundleIdentifier</key><string>dev.mallard.session-diagrams-qa.${fixtureRoot.split("-").at(-1)}</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1.6</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>`);
const output = resolve(appRoot, "output/mermaid-flowcharts");
await mkdir(output, { recursive: true });
await writeFile(resolve(output, "fixture-root.txt"), fixtureRoot);
await writeFile(resolve(output, "full-native-app-path.txt"), bundle);
console.log("Prepared the isolated application. Its path is in output/mermaid-flowcharts/full-native-app-path.txt.");

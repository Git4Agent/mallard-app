export const diagram = `flowchart TD
    A["Pull installs transcript<br/>Save byte offset N"] --> B["Continue and Push<br/>Keep offset N"]
    B --> C["Desktop import deletes thinking rows<br/>Record positions change"]
    C --> D["Refresh checks old offset N"]
    D --> E["Cutoff validation fails"]
    E --> F["Local evidence is invalid<br/>Show Unknown"]
`;
export const fenced = (code: string, language = "mermaid") => `\n\n\`\`\`${language}\n${code}\n\`\`\`\n`;

export function fixture(name: string) {
  const intro = "The flowchart below uses labels from the attached image. The recorded source stays portable.";
  const ordinary = fenced(diagram, "css") + fenced('console.log("A Mermaid fence is separate from JavaScript code.");', "javascript");
  const cases: Record<string, string> = {
    diagram: intro + fenced(diagram),
    multiple: "Two flowcharts in one message." + fenced("graph LR\nA[One] --> B[Two]") + fenced("flowchart TD\nA[Start] --> B{Ready?}\nB -->|Yes| C[Done]\nB -->|No| D[Wait]"),
    malformed: "An invalid Mermaid diagram." + fenced('flowchart TD\nA["Unclosed label'),
    empty: "The user's minimal placeholder." + fenced("flowchart"),
    unclosed: "An incomplete message fence.\n\n```mermaid\nflowchart TD\nA --> B",
    oversized: "A diagram over the source limit." + fenced("flowchart TD\n%% " + "x".repeat(32768)),
    ordinary: "Other code remains a code block." + ordinary,
    directives: "A diagram tries to enable HTML labels and clicks." + fenced('%%{init: {"securityLevel":"loose","htmlLabels":true,"themeCSS":"@import url(/canary);"}}%%\nflowchart LR\nA["Safe label"] --> B["Strict settings stay active"]\nclick A href "https://example.com/"'),
    frontmatter: "Frontmatter tries to change host settings." + fenced('---\nconfig:\n  securityLevel: loose\n  htmlLabels: true\n  themeCSS: "@import url(/canary);"\n---\nflowchart LR\nA[Safe] --> B[Strict]'),
    resources: "A diagram attempts to load an external image." + fenced('flowchart LR\nA@{ img: "http://127.0.0.1:64482/canary", label: "External image", h: 80 } --> B[Blocked]'),
    labels: "Untrusted markup in a node label." + fenced('flowchart TD\nA["<img src=\'/canary\' onerror=\'window.attack=true\'>"] --> B["<script>window.attack=true</script>"]'),
    nodes: "A flowchart over the 128-node limit." + fenced("flowchart TD\n" + Array.from({ length: 129 }, (_, index) => `N${index}[Node ${index}]`).join("\n")),
    edges: "A flowchart over the 128-edge limit." + fenced("flowchart TD\n" + "A --> B\n".repeat(129)),
    quoted: "> A flowchart inside a Markdown quote.\n>\n> ```mermaid\n> flowchart LR\n> A[Start] --> B[End]\n> ```",
  };
  return cases[name] ?? cases.diagram;
}

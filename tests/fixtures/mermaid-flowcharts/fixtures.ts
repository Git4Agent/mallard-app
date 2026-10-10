export const diagram = `flowchart TD
    A["Pull installs transcript<br/>Save byte offset N"] --> B["Continue and Push<br/>Keep offset N"]
    B --> C["Desktop import deletes thinking rows<br/>Record positions change"]
    C --> D["Refresh checks old offset N"]
    D --> E["Cutoff validation fails"]
    E --> F["Local evidence is invalid<br/>Show Unknown"]
`;
export const fenced = (code: string, language = "mermaid") => `\n\n\`\`\`${language}\n${code}\n\`\`\`\n`;

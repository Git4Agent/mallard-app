import type { Extension } from "mdast-util-from-markdown";
import type { Processor } from "unified";

declare module "unified" {
  interface Data {
    fromMarkdownExtensions?: Extension[];
  }
}

/** The Markdown tokenizer sees both fences, including inside lists and quotes. */
export function remarkMermaidFences(this: Processor): void {
  const extension: Extension = {
    enter: {
      codeFencedFence() {
        // The closing fence is inside the compiler's buffered code fragment.
        for (let index = this.stack.length - 1; index >= 0; index--) {
          const node = this.stack[index];
          if (node?.type !== "code") continue;
          node.data ??= {};
          const properties = node.data.hProperties ??= {};
          properties["data-mermaid-complete"] = properties["data-mermaid-complete"] === false;
          return;
        }
      },
    },
  };
  const extensions = this.data("fromMarkdownExtensions") ?? [];
  extensions.push(extension);
  this.data("fromMarkdownExtensions", extensions);
}

import type { Extension } from "mdast-util-from-markdown";
import type { Code, Extension as SyntaxExtension, State, Token, Tokenizer } from "micromark-util-types";
import type { Processor } from "unified";

declare module "unified" {
  interface Data {
    micromarkExtensions?: SyntaxExtension[];
  }
}

declare module "micromark-util-types" {
  interface TokenTypeMap {
    latexMath: "latexMath";
    latexMathMarker: "latexMathMarker";
  }
}

function isLineEnding(code: Code): boolean {
  return code === -5 || code === -4 || code === -3;
}

/** Parse TeX delimiters before Markdown escapes or emphasis can change them.
 * As a text construct this also works inside lists and quotes, while code
 * spans, code fences, and link destinations keep their literal contents. */
const tokenizeLatexMath: Tokenizer = function (effects, ok, nok) {
  let closing: number;
  let marker: Token;
  return start;

  function start(code: Code): State | undefined {
    effects.enter("latexMath");
    effects.enter("latexMathMarker");
    effects.consume(code);
    return open;
  }

  function open(code: Code): State | undefined {
    if (code !== 40 && code !== 91) return nok(code);
    closing = code === 40 ? 41 : 93;
    effects.consume(code);
    effects.exit("latexMathMarker");
    return between;
  }

  function between(code: Code): State | undefined {
    if (code === null) return nok(code);
    if (code === 92) {
      marker = effects.enter("latexMathMarker");
      effects.consume(code);
      return close;
    }
    if (isLineEnding(code)) {
      effects.enter("lineEnding");
      effects.consume(code);
      effects.exit("lineEnding");
      return between;
    }
    effects.enter("data");
    return data(code);
  }

  function data(code: Code): State | undefined {
    if (code === null || code === 92 || isLineEnding(code)) {
      effects.exit("data");
      return between(code);
    }
    effects.consume(code);
    return data;
  }

  function close(code: Code): State | undefined {
    if (code === closing) {
      effects.consume(code);
      effects.exit("latexMathMarker");
      effects.exit("latexMath");
      return ok;
    }
    // A TeX command or escaped backslash belongs to the formula. In particular,
    // the second slash of \\ must not start a closing delimiter.
    marker.type = "data";
    if (code === 92) {
      effects.consume(code);
      effects.exit("data");
      return between;
    }
    effects.exit("data");
    return between(code);
  }
};

export function remarkLatexMath(this: Processor): void {
  const syntax = this.data("micromarkExtensions") ?? [];
  syntax.push({ text: { 92: { name: "latexMath", tokenize: tokenizeLatexMath } } });
  this.data("micromarkExtensions", syntax);

  const extension: Extension = {
    enter: {
      latexMath(token) {
        this.enter({
          type: "inlineMath",
          value: "",
          data: {
            hName: "code",
            hProperties: {
              className: ["language-math", this.sliceSerialize(token).startsWith("\\[")
                ? "math-display" : "math-inline"],
            },
          },
        }, token);
        this.buffer();
      },
    },
    exit: {
      latexMath(token) {
        const value = this.resume();
        const node = this.stack[this.stack.length - 1];
        if (node.type === "inlineMath") {
          node.value = value;
          node.data!.hChildren = [{ type: "text", value }];
        }
        this.exit(token);
      },
    },
  };
  const extensions = this.data("fromMarkdownExtensions") ?? [];
  extensions.push(extension);
  this.data("fromMarkdownExtensions", extensions);
}

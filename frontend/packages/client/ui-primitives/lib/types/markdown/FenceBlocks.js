import { jsx as _jsx, Fragment as _Fragment } from "react/jsx-runtime";
import { CodeBlock } from "./CodeBlock.js";
import { renderTexToReact } from "./katex.js";
/**
 * Render one fence body as a code block.
 * @param props - fence language, body, streaming state, chrome labels, and diff choice.
 * @returns the code block.
 */
export function FenceCodeBlock({ lang, code, streaming, labels, diff }) {
    return (_jsx(CodeBlock
    // The replaced hast pipeline appended one synthetic newline that
    // CodeBlock's display trim removes; feeding the bare value would make
    // that trim eat a REAL trailing blank line inside the fence instead.
    , { 
        // The replaced hast pipeline appended one synthetic newline that
        // CodeBlock's display trim removes; feeding the bare value would make
        // that trim eat a REAL trailing blank line inside the fence instead.
        code: `${code}\n`, lang: lang, streaming: streaming, diff: diff, copyLabel: labels.copyLabel, copiedLabel: labels.copiedLabel, toolbarLabels: labels.toolbarLabels }));
}
/**
 * Render one `math` fence body as display TeX.
 * @param props - the fence body without its closing newline.
 * @returns the typeset formula.
 */
export function FenceMathBlock({ code }) {
    // rehype-katex parity: its text extraction saw the code block's trailing newline.
    return _jsx(_Fragment, { children: renderTexToReact(`${code}\n`, true) });
}
//# sourceMappingURL=FenceBlocks.js.map
import { jsx as _jsx } from "react/jsx-runtime";
/** Chat's language layers for Markdown fences: display math and unified-diff rows. */
import { useMemo } from 'react';
import { FenceCodeBlock, FenceMathBlock } from '@deepseek-ai/dsh-client-ui-primitives';
import { markdownLabels } from "../markdown-labels.js";
/**
 * Show a `math` fence as display TeX once its document settles; while it
 * streams, the inner code block shows the source so a partial formula never
 * renders as an error.
 * @param props - the fence and the inner content.
 * @returns the formula, or the inner content while streaming.
 */
export function MathFenceLayer({ code, streaming, children }) {
    return streaming ? children : _jsx(FenceMathBlock, { code: code });
}
/**
 * Show a `diff` or `patch` fence as unified-diff rows in place of the inner code block.
 * @param props - the fence and the Chat locale seat.
 * @returns the diff code block.
 */
export function DiffFenceLayer({ lang, code, streaming, t }) {
    const labels = useMemo(() => markdownLabels(t).code, [t]);
    return _jsx(FenceCodeBlock, { lang: lang, code: code, streaming: streaming, labels: labels, diff: true });
}
//# sourceMappingURL=FenceLayers.js.map
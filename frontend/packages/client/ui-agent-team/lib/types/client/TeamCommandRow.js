import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
/** Read-only historical /team failures and fallback output. */
import { useState } from 'react';
import { DisclosureRow, IconUsersOutlineRegular, TextShimmer } from '@deepseek-ai/dsh-client-ui-primitives';
import { assertNever } from '@deepseek-ai/dsh-util-values';
import css from './TeamCommandRow.module.css';
const TEAM_ICON = _jsx(IconUsersOutlineRegular, { size: 14 });
function commandSummary(node, t) {
    const outcome = node.outcome;
    if (outcome === null)
        return t('command.running');
    switch (outcome.kind) {
        case 'success':
            return outcome.text || t('command.done');
        case 'error': {
            const reason = outcome.text?.trim().split(/\r?\n/u)[0];
            return reason ? t('command.failedReason', { reason }) : t('command.failed');
        }
        /* v8 ignore next -- CommandNode restricts outcomes to success or error. */
        default: return assertNever(outcome.kind);
    }
}
/**
 * Render the recorded Team command with the shared agent glyph and disclosure.
 * @param props - command record and localized Team text.
 * @returns an outcome row; recorded failures and multiline output remain expandable.
 */
export function TeamCommandRow({ node, t }) {
    const [expanded, setExpanded] = useState(false);
    const running = node.outcome === null;
    const failed = node.outcome?.kind === 'error';
    const summary = commandSummary(node, t);
    const body = node.outcome?.text;
    const expandable = body !== undefined && body !== '' && (failed || body.includes('\n'));
    return (_jsx("div", { className: css.root, "data-team-command": true, "data-state": running ? 'running' : failed ? 'error' : 'ok', "aria-busy": running || undefined, children: _jsx(DisclosureRow, { icon: TEAM_ICON, title: t('command.title'), titleClassName: css.title, running: running, open: expanded && expandable, expandable: expandable, expandOnRowClick: true, keepContentWhenOpen: true, onToggle: () => { setExpanded(value => !value); }, collapsedContent: (_jsxs(_Fragment, { children: [_jsx("span", { className: css.separator, "data-shimmer-decoration": true, "aria-hidden": true }), _jsx("span", { className: css.summary, "data-error": failed || undefined, children: _jsx(TextShimmer, { children: summary }) })] })), children: _jsx("pre", { className: css.body, "data-error": failed || undefined, children: body }) }) }));
}
//# sourceMappingURL=TeamCommandRow.js.map
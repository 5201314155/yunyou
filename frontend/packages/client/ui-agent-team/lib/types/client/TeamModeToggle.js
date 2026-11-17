import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** Session-owned Agent Teams selection in the model and reasoning menu. */
import { useState } from 'react';
import { Button, IconInfoOutlineRegular, Switch, Tooltip } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './TeamModeToggle.module.css';
/**
 * Render the saved selection without an optimistic update.
 * @param props - Session projection, menu state, localized copy, and selection callback.
 * @returns the Team switch when the capability projection is available.
 */
export function TeamModeToggle(props) {
    const addressedChild = props.useSession(snapshot => snapshot.subagent !== null);
    const listedChild = props.useSessions(state => state.byId[props.sessionId]?.origin === 'subagent');
    if (addressedChild || listedChild)
        return null;
    return _jsx(RootTeamModeToggle, { ...props });
}
/** Read and change the selection only for root Sessions. */
function RootTeamModeToggle({ useProjection, locked, dismiss, reportFailure, selectTeamMode, requestDisableConfirmation, t, }) {
    const mode = useProjection('agentTeam');
    const modelSelection = useProjection('modelSelection');
    const [pending, setPending] = useState(false);
    if (mode === undefined)
        return null;
    const select = (enabled) => {
        if (!enabled && modelSelection !== undefined && modelSelection.lastUsed !== null) {
            dismiss();
            requestDisableConfirmation();
            return;
        }
        setPending(true);
        void selectTeamMode(enabled).then((failure) => {
            if (failure !== null)
                reportFailure(t('mode.failed', { message: failure }));
        }, (error) => {
            reportFailure(t('mode.failed', { message: error instanceof Error ? error.message : String(error) }));
        }).finally(() => { setPending(false); });
    };
    return (_jsxs("div", { className: css.row, children: [_jsxs("span", { className: css.label, children: [_jsx("span", { children: t('mode.label') }), _jsx(Tooltip, { label: t('mode.description'), side: "top", delayMs: 300, maxWidth: 300, portal: true, openOnClick: true, children: _jsx(Button, { variant: "ghost", size: "sm", className: css.infoButton, "aria-label": t('mode.infoLabel'), children: _jsx(IconInfoOutlineRegular, { size: 11, "aria-hidden": "true" }) }) })] }), _jsx(Switch, { className: css.toggle, checked: mode.enabled, disabled: locked || pending || modelSelection === undefined, label: t('mode.label'), onChange: select })] }));
}
//# sourceMappingURL=TeamModeToggle.js.map
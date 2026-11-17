import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
/** Session-targeted confirmation that outlives the model menu which opened it. */
import { useState } from 'react';
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './TeamDisableConfirmation.module.css';
/**
 * Render the requested confirmation outside the model menu.
 * @param props - request observation, command callbacks, and localized text.
 * @returns the confirmation, or nothing when no request is pending.
 */
export function TeamDisableConfirmation({ useDisableRequest, ...props }) {
    const request = useDisableRequest(value => value);
    if (request === null)
        return null;
    return _jsx(DisableConfirmationForm, { sessionId: request.sessionId, ...props }, request.sessionId);
}
/** One confirmation keeps its failure visible until retry or cancellation. */
function DisableConfirmationForm({ sessionId, cancelDisable, disableTeam, t }) {
    const [state, setState] = useState({ kind: 'ready' });
    const saving = state.kind === 'saving';
    const cancel = () => {
        if (!saving)
            cancelDisable(sessionId);
    };
    const confirm = () => {
        if (saving)
            return;
        setState({ kind: 'saving' });
        void disableTeam(sessionId).then((failure) => {
            if (failure === null)
                cancelDisable(sessionId);
            else
                setState({ kind: 'failed', message: failure });
        }, (error) => {
            setState({ kind: 'failed', message: error instanceof Error ? error.message : String(error) });
        });
    };
    return (_jsxs(Modal, { open: true, tone: "warning", title: t('disable.title'), closeLabel: t('disable.close'), onClose: cancel, className: css.dialog ?? '', contentClassName: css.content ?? '', footer: (_jsxs(_Fragment, { children: [_jsx(Button, { variant: "outline", disabled: saving, "data-modal-autofocus": true, onClick: cancel, children: t('disable.cancel') }), _jsx(Button, { variant: "primary", disabled: saving, onClick: confirm, children: t('disable.confirm') })] })), children: [_jsxs("div", { className: css.copy, children: [_jsx("p", { children: t('disable.description') }), _jsx("p", { children: t('disable.warning') })] }), state.kind === 'failed' && _jsx("p", { role: "alert", className: css.error, children: t('mode.failed', { message: state.message }) })] }));
}
//# sourceMappingURL=TeamDisableConfirmation.js.map
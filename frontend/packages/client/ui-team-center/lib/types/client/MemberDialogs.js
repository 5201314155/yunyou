import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 成员操作对话框组：发消息（L2 直通）与踢人（L3 风险确认）。 */
import { useState } from 'react';
import { Button, Modal, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './MemberDialogs.module.css';
/**
 * 渲染当前成员对话框；同一时刻至多一个。踢人的主操作在勾选确认前置不可用。
 * @param props - 本地化、当前对话框与回调。
 * @returns 对话框组（无对话框时渲染 null）。
 */
export function MemberDialogs({ t, dialog, onClose, onKick, onMessage }) {
    const [acknowledged, setAcknowledged] = useState(false);
    const [body, setBody] = useState('');
    const [pending, setPending] = useState(false);
    const close = () => {
        setAcknowledged(false);
        setBody('');
        setPending(false);
        onClose();
    };
    if (dialog === undefined)
        return null;
    if (dialog.kind === 'kick') {
        return (_jsx(RiskConfirmation, { open: true, title: t('confirm.kick.title', { name: dialog.name }), description: t('confirm.kick.body'), acknowledgeLabel: t('confirm.kick.acknowledge'), cancelLabel: t('confirm.cancel'), closeLabel: t('close'), confirmLabel: t('confirm.submit'), acknowledged: acknowledged, disabled: pending, onAcknowledgedChange: setAcknowledged, onCancel: close, onConfirm: () => {
                setPending(true);
                void onKick(dialog.name).then(close);
            } }));
    }
    return (_jsx(Modal, { open: true, onClose: close, title: t('message.title', { name: dialog.name }), closeLabel: t('close'), ...(css.sheet ? { className: css.sheet } : {}), children: _jsxs("form", { className: css.form, onSubmit: (event) => {
                event.preventDefault();
                if (body.trim() === '' || pending)
                    return;
                setPending(true);
                void onMessage(dialog.name, body).then(close);
            }, children: [_jsx("h2", { className: css.title, children: t('message.body') }), _jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('message.body') }), _jsx("textarea", { className: css.body, value: body, onChange: event => { setBody(event.target.value); }, rows: 4, required: true })] }), _jsxs("div", { className: css.actions, children: [_jsx(Button, { variant: "ghost", type: "button", onClick: close, children: t('message.cancel') }), _jsx(Button, { variant: "primary", type: "submit", disabled: pending || body.trim() === '', children: t('message.send') })] })] }) }));
}
//# sourceMappingURL=MemberDialogs.js.map
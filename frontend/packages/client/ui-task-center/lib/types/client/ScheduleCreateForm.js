import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 定时计划创建表单：三选一必填恰一+nextFireMs 回显确认（契约 §3）。 */
import { useState } from 'react';
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './ScheduleCreateForm.module.css';
/**
 * 渲染创建表单：单次/间隔/cron 三选一；提交即回显 nextFireMs（响应即确认步，
 * 服务端 nowMs+nextFireMs 为准；durable 无跨重启开关）。
 * @param props - 本地化、开合与远程源。
 * @returns 创建表单。
 */
export function ScheduleCreateForm({ t, open, onClose, source, onCreated }) {
    const [kind, setKind] = useState('once');
    const [atLocal, setAtLocal] = useState('');
    const [every, setEvery] = useState('5');
    const [cron, setCron] = useState('');
    const [message, setMessage] = useState('');
    const [pending, setPending] = useState(false);
    const close = () => { setPending(false); onClose(); };
    if (!open)
        return null;
    const request = () => {
        if (message.trim() === '')
            return undefined;
        if (kind === 'once') {
            const at = Date.parse(atLocal);
            return Number.isNaN(at) ? undefined : { at_ms: at, message };
        }
        if (kind === 'interval') {
            const minutes = Number(every);
            return Number.isInteger(minutes) && minutes >= 5 ? { every_minutes: minutes, message } : undefined;
        }
        return cron.trim() === '' ? undefined : { cron: cron.trim(), message };
    };
    return (_jsx(Modal, { open: true, onClose: close, title: t('schedule.create'), closeLabel: t('close'), ...(css.sheet ? { className: css.sheet } : {}), children: _jsxs("form", { className: css.form, onSubmit: (event) => {
                event.preventDefault();
                const body = request();
                if (pending || body === undefined)
                    return;
                setPending(true);
                void source.scheduleCreate(body).then(result => {
                    setPending(false);
                    if (!result.ok)
                        return;
                    onCreated();
                    close();
                });
            }, children: [_jsxs("fieldset", { className: css.field, children: [_jsx("legend", { className: css.fieldLabel, children: t('schedule.kind') }), ['once', 'interval', 'cron'].map(name => (_jsxs("label", { className: css.radio, children: [_jsx("input", { type: "radio", name: "schedule-kind", checked: kind === name, onChange: () => { setKind(name); } }), _jsx("span", { children: t(`schedule.kind.${name}`) })] }, name)))] }), kind === 'once' ? (_jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('schedule.at') }), _jsx("input", { className: css.input, type: "datetime-local", value: atLocal, onChange: e => { setAtLocal(e.target.value); }, required: true })] })) : null, kind === 'interval' ? (_jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('schedule.every') }), _jsx("input", { className: css.input, type: "number", min: 5, step: 1, value: every, onChange: e => { setEvery(e.target.value); }, required: true })] })) : null, kind === 'cron' ? (_jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('schedule.cron') }), _jsx("input", { className: css.input, value: cron, onChange: e => { setCron(e.target.value); }, required: true })] })) : null, _jsxs("label", { className: css.field, children: [_jsx("span", { className: css.fieldLabel, children: t('schedule.message') }), _jsx("textarea", { className: css.body, value: message, onChange: e => { setMessage(e.target.value); }, rows: 3, required: true })] }), _jsx("p", { className: css.note, children: t('schedule.durableNote') }), _jsxs("div", { className: css.actions, children: [_jsx(Button, { variant: "ghost", type: "button", onClick: close, children: t('schedule.cancel') }), _jsx(Button, { variant: "primary", type: "submit", disabled: pending, children: t('schedule.confirm') })] })] }) }));
}
//# sourceMappingURL=ScheduleCreateForm.js.map
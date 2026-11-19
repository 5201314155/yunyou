import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 定时计划列表：id/下次触发/消息三列与删除二次确认（L3）。 */
import { useState } from 'react';
import { Button, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './ScheduleList.module.css';
/**
 * 渲染定时计划列表；删除走 RiskConfirmation（契约 §3 L3）。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 定时计划列表。
 */
export function ScheduleList({ t, phase, entries, source, notify }) {
    const [confirm, setConfirm] = useState(undefined);
    const [acknowledged, setAcknowledged] = useState(false);
    const runDelete = () => {
        if (confirm === undefined)
            return;
        void source.scheduleDelete(confirm.id).then(result => {
            if (!result.ok)
                notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
            else
                notify(`${t('schedule.delete')} · ${confirm.id}`, 'info');
            setConfirm(undefined);
            setAcknowledged(false);
        });
    };
    return (_jsxs("div", { className: css.list, children: [phase === 'ready' && entries.length === 0 ? _jsx("p", { className: css.empty, children: t('schedule.empty') })
                : phase === 'ready' ? (_jsx("ul", { className: css.rows, children: entries.map(entry => (_jsxs("li", { className: css.row, children: [_jsx("span", { className: css.id, title: entry.id, children: entry.id }), _jsx("span", { className: css.next, children: new Date(entry.nextFireMs).toLocaleString() }), _jsx("span", { className: css.message, title: entry.message, children: entry.message }), _jsx(Button, { variant: "toolbar", onClick: () => { setConfirm(entry); setAcknowledged(false); }, children: t('schedule.delete') })] }, entry.id))) })) : _jsx("p", { className: css.empty, children: t('page.unavailable') }), confirm !== undefined ? (_jsx(RiskConfirmation, { open: true, title: t('schedule.deleteConfirm', { id: confirm.id }), description: t('schedule.durableNote'), acknowledgeLabel: t('schedule.deleteAck'), cancelLabel: t('confirm.cancel'), closeLabel: t('close'), confirmLabel: t('confirm.submit'), acknowledged: acknowledged, onAcknowledgedChange: setAcknowledged, onCancel: () => { setConfirm(undefined); }, onConfirm: runDelete })) : null] }));
}
//# sourceMappingURL=ScheduleList.js.map
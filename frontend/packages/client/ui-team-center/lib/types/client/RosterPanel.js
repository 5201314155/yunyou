import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 名册栏：成员在线/忙碌/当前任务与 L2–L3 操作入口。 */
import { useState } from 'react';
import { Button, StateDot } from '@deepseek-ai/dsh-client-ui-primitives';
import { MemberDialogs } from "./MemberDialogs.js";
import css from './RosterPanel.module.css';
/** 成员可用性点：busy 优先于 online（服务端合成语义）。 */
function availability(entry) {
    if (entry.busy)
        return { state: 'ongoing', key: 'roster.busy' };
    if (entry.online)
        return { state: 'done', key: 'roster.online' };
    return { state: 'idle', key: 'roster.offline' };
}
/**
 * 渲染名册栏：每行状态点+名字+当前任务；行内操作触发 L2/L3 确认流。
 * @param props - 快照、阶段、回调与本地化。
 * @returns 名册栏。
 */
export function RosterPanel({ t, phase, roster, onSpawn, source, notify }) {
    const [dialog, setDialog] = useState(undefined);
    const runKick = async (name) => {
        const result = await source.kick(name);
        if (!result.ok)
            notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
        else
            notify(`${t('roster.kick')} ${name}`, 'info');
    };
    const runMessage = async (name, body) => {
        const result = await source.message({ to: name, body });
        if (!result.ok)
            notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
        else
            notify(`${t('message.send')} → ${name}`, 'info');
    };
    return (_jsxs("section", { className: css.panel, "aria-label": t('roster.title'), children: [_jsxs("header", { className: css.panelHeader, children: [_jsx("h2", { className: css.panelTitle, children: t('roster.title') }), _jsx(Button, { variant: "ghost", onClick: onSpawn, children: t('roster.spawn') })] }), phase === 'ready' && roster.length === 0 ? _jsx("p", { className: css.empty, children: t('roster.empty') })
                : phase === 'ready' ? (_jsx("ul", { className: css.rows, children: roster.map(entry => {
                        const availabilityState = availability(entry);
                        return (_jsxs("li", { className: css.row, children: [_jsxs("span", { className: css.availability, "data-state": availabilityState.state, children: [_jsx(StateDot, { state: availabilityState.state }), _jsx("span", { className: css.availabilityText, children: t(availabilityState.key) })] }), _jsx("span", { className: css.name, title: entry.description, children: entry.name }), _jsx("span", { className: css.task, children: entry.task ?? '' }), _jsxs("span", { className: css.rowActions, children: [_jsx(Button, { variant: "toolbar", onClick: () => { setDialog({ kind: 'message', name: entry.name }); }, children: t('roster.message') }), _jsx(Button, { variant: "toolbar", onClick: () => { setDialog({ kind: 'kick', name: entry.name }); }, children: t('roster.kick') })] })] }, entry.name));
                    }) })) : _jsx("p", { className: css.unavailable, children: t('page.unavailable') }), _jsx(MemberDialogs, { t: t, dialog: dialog, onClose: () => { setDialog(undefined); }, onKick: runKick, onMessage: runMessage })] }));
}
//# sourceMappingURL=RosterPanel.js.map
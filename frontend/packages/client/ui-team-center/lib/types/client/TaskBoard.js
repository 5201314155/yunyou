import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 共享任务板：状态机行列表与 L2 写操作/L3 审查确认（写审分离面）。 */
import { useState } from 'react';
import { Button, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './TaskBoard.module.css';
/** 状态徽标语义色（不只凭颜色传达：文案并行）。 */
const STATE_TONE = {
    pending: 'idle',
    claimed: 'ongoing',
    submitted: 'ongoing',
    reviewing: 'ongoing',
    approved: 'done',
    rejected: 'error',
};
/** 行上允许的下一动作（状态机映射，契约 §2.1）。 */
function availableActions(task) {
    switch (task.state) {
        case 'pending': return ['claim'];
        case 'claimed': return ['submit'];
        case 'submitted':
        case 'reviewing': return ['approve', 'reject'];
        default: return [];
    }
}
/**
 * 渲染任务板：行内状态+负责人+阻塞来源；操作走真实 RPC，审查走风险确认。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 任务板。
 */
export function TaskBoard({ t, phase, tasks, source, notify }) {
    const [confirm, setConfirm] = useState(undefined);
    const [pending, setPending] = useState(false);
    const act = (task, action) => {
        setPending(true);
        const call = action === 'claim' ? source.claim(task.id) : source.submit(task.id);
        void call.then(result => {
            setPending(false);
            if (!result.ok)
                notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
            else
                notify(t(`task.${action}`) + ' · ' + task.title, 'info');
        });
    };
    const review = (task, approve) => {
        setPending(true);
        void source.review(task.id, approve).then(result => {
            setPending(false);
            if (!result.ok)
                notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error');
            else
                notify(t(approve ? 'state.approved' : 'state.rejected') + ' · ' + task.title, 'info');
        });
    };
    return (_jsxs("section", { className: css.panel, "aria-label": t('tasks.title'), children: [_jsx("header", { className: css.panelHeader, children: _jsx("h2", { className: css.panelTitle, children: t('tasks.title') }) }), phase === 'ready' && tasks.length === 0 ? _jsx("p", { className: css.empty, children: t('tasks.empty') })
                : phase === 'ready' ? (_jsx("ul", { className: css.rows, children: tasks.map(task => (_jsx(TaskRowContent, { t: t, task: task, pending: pending, onAct: act, onReview: (approve) => { setConfirm({ kind: 'review', task, approve }); } }, task.id))) })) : _jsx("p", { className: css.unavailable, children: t('page.unavailable') }), confirm !== undefined ? (_jsx(RiskConfirmation, { open: true, title: t('confirm.review.title', { title: confirm.task.title }), description: t('confirm.review.body'), acknowledgeLabel: t('confirm.review.acknowledge'), cancelLabel: t('confirm.cancel'), closeLabel: t('close'), confirmLabel: t('confirm.submit'), acknowledged: false, disabled: pending, onAcknowledgedChange: () => { setConfirm({ ...confirm }); }, onCancel: () => { setConfirm(undefined); }, onConfirm: () => { review(confirm.task, confirm.approve); setConfirm(undefined); } })) : null] }));
}
/** 单行内容（拆出保持页面组件行数纪律）。 */
function TaskRowContent({ t, task, pending, onAct, onReview }) {
    const actions = availableActions(task);
    const stateKey = `state.${task.state}`;
    return (_jsxs("li", { className: css.row, "data-state": task.state, children: [_jsxs("div", { className: css.rowMain, children: [_jsx("span", { className: css.stateBadge, "data-tone": STATE_TONE[task.state], children: t(stateKey) }), _jsx("span", { className: css.title, title: task.title, children: task.title }), _jsxs("span", { className: css.meta, children: [task.owner === undefined ? '' : `@${task.owner}`, task.capability === undefined ? '' : ` · ${task.capability}`] }), task.blockedBy.length > 0 ? (_jsx("span", { className: css.blocked, children: t('tasks.blockedBy', { count: task.blockedBy.length }) })) : null] }), actions.length > 0 ? (_jsxs("span", { className: css.rowActions, children: [actions.includes('claim') ? (_jsx(Button, { variant: "toolbar", disabled: pending, onClick: () => { onAct(task, 'claim'); }, children: t('task.claim') })) : null, actions.includes('submit') ? (_jsx(Button, { variant: "toolbar", disabled: pending, onClick: () => { onAct(task, 'submit'); }, children: t('task.submit') })) : null, actions.includes('approve') ? (_jsx(Button, { variant: "toolbar", disabled: pending, onClick: () => { onReview(true); }, children: t('task.review.approve') })) : null, actions.includes('reject') ? (_jsx(Button, { variant: "toolbar", disabled: pending, onClick: () => { onReview(false); }, children: t('task.review.reject') })) : null] })) : null] }));
}
//# sourceMappingURL=TaskBoard.js.map
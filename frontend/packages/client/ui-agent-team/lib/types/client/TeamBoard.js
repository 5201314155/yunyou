import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { useState } from 'react';
import { IconChecklistOutlineRegular, IconChevronDownOutlineRegular, StateDot, Tooltip, } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './TeamBoard.module.css';
const statusKeys = {
    pending: 'status.pending', in_progress: 'status.in_progress', completed: 'status.completed',
};
const dotStates = {
    pending: 'idle', in_progress: 'ongoing', completed: 'done',
};
function progressLabel(tasks, t) {
    const completed = tasks.filter(task => task.status === 'completed').length;
    const active = tasks.filter(task => task.status === 'in_progress').length;
    const pending = tasks.length - completed - active;
    return [
        ...completed > 0 ? [t('progress.completed', { count: completed })] : [],
        ...active > 0 ? [t('progress.active', { count: active })] : [],
        ...pending > 0 ? [t('progress.pending', { count: pending })] : [],
    ].join('\u2002·\u2002');
}
function TaskRow({ task, rootId, expanded, onToggle, t }) {
    const owner = task.ownerName ?? task.ownerId ?? t('unowned');
    const blocked = task.status === 'pending' && !task.ready;
    const detailsId = `team-task-${rootId}-${task.id}`;
    return (_jsxs("li", { className: css.task, "data-team-task": task.id, "data-status": task.status, "data-expanded": expanded, children: [_jsx(Tooltip, { label: task.subject, onlyWhenClipped: true, disabled: expanded, delayMs: 500, portal: true, side: "top", children: _jsxs("button", { type: "button", className: css.taskRow, "aria-expanded": expanded, "aria-controls": detailsId, onClick: onToggle, children: [_jsx("span", { className: css.glyph, role: "img", "aria-label": blocked ? t('blocked') : t(statusKeys[task.status]), children: _jsx(StateDot, { state: blocked ? 'warning' : dotStates[task.status] }) }), _jsx("span", { className: css.subject, children: task.subject }), _jsx("span", { className: css.chevron, "aria-hidden": true, children: _jsx(IconChevronDownOutlineRegular, { size: 14 }) })] }) }), expanded && (_jsxs("div", { className: css.details, id: detailsId, "data-team-task-details": true, children: [task.description && _jsx("p", { className: css.description, children: task.description }), _jsxs("p", { className: css.owner, "data-team-task-owner": true, children: [_jsxs("span", { className: css.ownerLabel, children: [t('owner'), ": "] }), _jsx(Tooltip, { label: owner, onlyWhenClipped: true, delayMs: 500, portal: true, side: "top", children: _jsx("span", { className: css.ownerValue, children: owner }) })] })] }))] }));
}
/** Render read-only tasks in source order; the enclosing subagent menu owns scrolling. */
export function TeamBoard({ sessionId, useProjection, useSessions, t }) {
    const team = useProjection('agentTeam');
    const hasMembers = useSessions(state => (state.projectionsBySession[sessionId]?.values.subagentCatalog?.length ?? 0) > 0);
    const [expandedTaskId, setExpandedTaskId] = useState(null);
    if (!team?.enabled)
        return null;
    const tasks = team.tasks.filter((task) => task.status !== 'deleted');
    const failure = team.failure === undefined ? null : (_jsxs("div", { className: css.error, role: "alert", children: [_jsx(StateDot, { state: "error" }), t('failure', { message: team.failure })] }));
    if (tasks.length === 0)
        return failure === null ? null : _jsx("div", { className: css.board, children: failure });
    return (_jsxs("section", { className: css.board, "data-with-members": hasMembers, "aria-label": t('trigger'), "data-team-panel": true, children: [_jsxs("h3", { className: css.header, children: [_jsx("span", { className: css.lead, "aria-hidden": true, children: _jsx(IconChecklistOutlineRegular, { size: 14 }) }), _jsx("span", { className: css.title, children: t('tasks') }), _jsx("span", { className: css.progress, children: progressLabel(tasks, t) })] }), failure, _jsx("ul", { className: css.tasks, children: tasks.map(task => (_jsx(TaskRow, { task: task, rootId: sessionId, expanded: expandedTaskId === task.id, onToggle: () => { setExpandedTaskId(current => current === task.id ? null : task.id); }, t: t }, task.id))) })] }));
}
//# sourceMappingURL=TeamBoard.js.map
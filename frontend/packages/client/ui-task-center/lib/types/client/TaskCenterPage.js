import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 任务中心主面板：共享任务/定时计划/后台任务三 tab（web.v1 tasks/schedule 域）。 */
import { useEffect, useState } from 'react';
import { Button } from '@deepseek-ai/dsh-client-ui-primitives';
import { ScheduleList } from "./ScheduleList.js";
import { ScheduleCreateForm } from "./ScheduleCreateForm.js";
import css from './TaskCenterPage.module.css';
/**
 * 渲染任务中心：tab 切换共享任务/定时计划/后台任务指引。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 任务中心页。
 */
export function TaskCenterPage(props) {
    const { useStore, actions, source, t } = props;
    const notify = notifyOf(actions);
    const [tab, setTab] = useState('shared');
    const [createOpen, setCreateOpen] = useState(false);
    const tasks = useStore(state => state.tasks);
    const tasksPhase = useStore(state => state.tasksPhase);
    const schedule = useStore(state => state.schedule);
    const schedulePhase = useStore(state => state.schedulePhase);
    const refresh = () => {
        actions.setTasks('loading', []);
        actions.setSchedule('loading', []);
        void source.tasks().then(result => {
            actions.setTasks(result.ok ? 'ready' : 'error', result.ok ? result.value.items : []);
        });
        void source.scheduleList().then(result => {
            actions.setSchedule(result.ok ? 'ready' : 'error', result.ok ? result.value.entries : []);
        });
    };
    useEffect(() => { refresh(); }, [refresh]);
    return (_jsxs("div", { className: css.page, children: [_jsxs("header", { className: css.header, children: [_jsx("h1", { className: css.title, children: t('page.title') }), _jsx("div", { className: css.tabs, role: "tablist", children: ['shared', 'schedule', 'jobs'].map(name => (_jsx("button", { type: "button", role: "tab", "aria-selected": tab === name, className: css.tab, onClick: () => { setTab(name); }, children: t(`tab.${name}`) }, name))) }), _jsx(Button, { variant: "ghost", onClick: refresh, children: t('page.refresh') })] }), tab === 'shared' ? (_jsx("section", { className: css.panel, "aria-label": t('tab.shared'), children: tasksPhase === 'ready' && tasks.length === 0 ? _jsx("p", { className: css.empty, children: t('tasks.empty') })
                    : tasksPhase === 'ready' ? (_jsx("ul", { className: css.rows, children: tasks.map(task => (_jsxs("li", { className: css.row, children: [_jsx("span", { className: css.badge, children: t(taskStateKey(task.state)) }), _jsx("span", { className: css.title, children: task.title }), _jsx("span", { className: css.meta, children: task.owner === undefined ? '' : `@${task.owner}` })] }, task.id))) })) : _jsx("p", { className: css.empty, children: t('page.unavailable') }) })) : null, tab === 'schedule' ? (_jsxs("section", { className: css.panel, "aria-label": t('tab.schedule'), children: [_jsx("div", { className: css.panelHeader, children: _jsx(Button, { variant: "ghost", onClick: () => { setCreateOpen(true); }, children: t('schedule.create') }) }), _jsx(ScheduleList, { t: t, phase: schedulePhase, entries: schedule, source: source, notify: notify }), _jsx(ScheduleCreateForm, { t: t, open: createOpen, onClose: () => { setCreateOpen(false); }, source: source, onCreated: refresh })] })) : null, tab === 'jobs' ? _jsx("p", { className: css.empty, children: t('jobs.empty') }) : null] }));
}
/** 状态键显式映射（locale 键类型锁）。 */
function taskStateKey(state) {
    return `state.${state}`;
}
/** store 动作折叠为通知回调。 */
function notifyOf(actions) {
    return (text, level) => { actions.setNotice({ text, level }); };
}
//# sourceMappingURL=TaskCenterPage.js.map
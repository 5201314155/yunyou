import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 团队中心主面板：名册与共享任务板分栏（web.v1 team/tasks 域产品面）。 */
import { useEffect, useState } from 'react';
import { Button } from '@deepseek-ai/dsh-client-ui-primitives';
import { RosterPanel } from "./RosterPanel.js";
import { SpawnDialog } from "./SpawnDialog.js";
import { TaskBoard } from "./TaskBoard.js";
import css from './TeamCenterPage.module.css';
/**
 * 渲染团队中心：左名册右任务板；接口未接时两栏显示诚实不可用态。
 * @param props - 框架快照、注入回调与本地化文案。
 * @returns 团队中心页面。
 */
export function TeamCenterPage(props) {
    const { useStore, actions, source, t } = props;
    const notify = (text, level) => { actions.setNotice({ text, level }); };
    const refresh = () => {
        actions.setRoster('loading', []);
        actions.setTasks('loading', []);
        void source.roster().then(result => {
            if (result.ok)
                actions.setRoster('ready', result.value);
            else
                actions.setRoster('error', []);
        });
        void source.tasks().then(result => {
            if (result.ok)
                actions.setTasks('ready', result.value);
            else
                actions.setTasks('error', []);
        });
    };
    const [spawnOpen, setSpawnOpen] = useState(false);
    const roster = useStore(state => state.roster);
    const rosterPhase = useStore(state => state.rosterPhase);
    const tasks = useStore(state => state.tasks);
    const tasksPhase = useStore(state => state.tasksPhase);
    // 首挂载拉一次真实数据；后续靠「刷新」与阶段 2 的事件重拉。
    useEffect(() => { refresh(); }, [refresh]);
    return (_jsxs("div", { className: css.page, children: [_jsxs("header", { className: css.header, children: [_jsx("h1", { className: css.title, children: t('page.title') }), _jsx(Button, { variant: "ghost", onClick: refresh, children: t('page.refresh') })] }), _jsxs("div", { className: css.columns, children: [_jsx(RosterPanel, { t: t, phase: rosterPhase, roster: roster, onSpawn: () => { setSpawnOpen(true); }, source: source, notify: notify }), _jsx(TaskBoard, { t: t, phase: tasksPhase, tasks: tasks, source: source, notify: notify })] }), _jsx(SpawnDialog, { t: t, open: spawnOpen, onClose: () => { setSpawnOpen(false); }, source: source, notify: notify })] }));
}
//# sourceMappingURL=TeamCenterPage.js.map
/** 团队中心查看态：名册/任务板快照与一次性提示；业务数据不落 store 之外。 */
import { defineStore } from '@deepseek-ai/dsh-client-store';
const teamActions = {
    setRoster: (state, phase, roster) => {
        state.rosterPhase = phase;
        state.roster = roster;
    },
    setTasks: (state, phase, tasks) => {
        state.tasksPhase = phase;
        state.tasks = tasks;
    },
    setNotice: (state, notice) => { state.notice = notice; },
};
/**
 * 声明团队中心的跨面板查看态（框架注册：store 由 main 挂载点持有）。
 * @returns 团队 store 句柄。
 */
export function createTeamStore() {
    return defineStore({
        init: () => ({ rosterPhase: 'idle', roster: [], tasksPhase: 'idle', tasks: [], notice: null }),
        actions: teamActions,
    });
}
//# sourceMappingURL=stores.js.map
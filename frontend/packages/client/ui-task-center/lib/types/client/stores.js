/** 任务中心查看态：任务板与定时计划快照。 */
import { defineStore } from '@deepseek-ai/dsh-client-store';
const actions = {
    setTasks: (state, phase, tasks) => {
        state.tasksPhase = phase;
        state.tasks = tasks;
    },
    setSchedule: (state, phase, schedule) => {
        state.schedulePhase = phase;
        state.schedule = schedule;
    },
    setNotice: (state, notice) => { state.notice = notice; },
};
/**
 * 声明任务中心查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export function createTaskCenterStore() {
    return defineStore({ init: () => ({ tasksPhase: 'idle', tasks: [], schedulePhase: 'idle', schedule: [], notice: null }), actions });
}
//# sourceMappingURL=stores.js.map
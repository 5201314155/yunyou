/** 任务中心查看态：任务板与定时计划快照。 */
import { type EngineStoreHandle } from '@deepseek-ai/dsh-client-store';
import type { LoadPhase, ScheduleListEntry, TaskEntry } from './contract.ts';
/** 一次性操作回执。 */
export interface TaskCenterNotice {
    readonly text: string;
    readonly level: 'info' | 'error';
}
export interface TaskCenterData {
    tasksPhase: LoadPhase;
    tasks: readonly TaskEntry[];
    schedulePhase: LoadPhase;
    schedule: readonly ScheduleListEntry[];
    notice: TaskCenterNotice | null;
}
declare const actions: {
    setTasks: (state: TaskCenterData, phase: LoadPhase, tasks: readonly TaskEntry[]) => void;
    setSchedule: (state: TaskCenterData, phase: LoadPhase, schedule: readonly ScheduleListEntry[]) => void;
    setNotice: (state: TaskCenterData, notice: TaskCenterNotice | null) => void;
};
/**
 * 声明任务中心查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export declare function createTaskCenterStore(): EngineStoreHandle<TaskCenterData, typeof actions>;
export type TaskCenterStoreHandle = ReturnType<typeof createTaskCenterStore>;
export {};
//# sourceMappingURL=stores.d.ts.map
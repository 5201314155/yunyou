/** 团队中心查看态：名册/任务板快照与一次性提示；业务数据不落 store 之外。 */
import { type EngineStoreHandle } from '@deepseek-ai/dsh-client-store';
import type { LoadPhase, RosterEntry, TaskEntry } from './contract.ts';
/** 一次性操作回执（人话，展示后清除）；level 决定语气。 */
export interface TeamNotice {
    readonly text: string;
    readonly level: 'info' | 'error';
}
export interface TeamData {
    rosterPhase: LoadPhase;
    roster: readonly RosterEntry[];
    tasksPhase: LoadPhase;
    tasks: readonly TaskEntry[];
    /** 阶段 2 事件重拉（tasks/changed、team/roster-changed）的接缝标记。 */
    notice: TeamNotice | null;
}
declare const teamActions: {
    setRoster: (state: TeamData, phase: LoadPhase, roster: readonly RosterEntry[]) => void;
    setTasks: (state: TeamData, phase: LoadPhase, tasks: readonly TaskEntry[]) => void;
    setNotice: (state: TeamData, notice: TeamNotice | null) => void;
};
/**
 * 声明团队中心的跨面板查看态（框架注册：store 由 main 挂载点持有）。
 * @returns 团队 store 句柄。
 */
export declare function createTeamStore(): EngineStoreHandle<TeamData, typeof teamActions>;
export type TeamStoreHandle = ReturnType<typeof createTeamStore>;
export {};
//# sourceMappingURL=stores.d.ts.map
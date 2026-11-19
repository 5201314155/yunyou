import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { LoadPhase, ScheduleListEntry, TaskCenterSource } from './contract.ts';
/** 列表 props。 */
export type ScheduleListProps = PropsLocale<'taskCenter'> & {
    readonly phase: LoadPhase;
    readonly entries: readonly ScheduleListEntry[];
    readonly source: TaskCenterSource;
    readonly notify: (text: string, level: 'info' | 'error') => void;
};
/**
 * 渲染定时计划列表；删除走 RiskConfirmation（契约 §3 L3）。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 定时计划列表。
 */
export declare function ScheduleList({ t, phase, entries, source, notify }: ScheduleListProps): import("react").JSX.Element;
//# sourceMappingURL=ScheduleList.d.ts.map
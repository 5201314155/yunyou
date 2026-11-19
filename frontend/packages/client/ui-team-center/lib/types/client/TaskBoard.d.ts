import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { LoadPhase, TaskEntry, TeamSource } from './contract.ts';
/** 任务板 props。 */
export type TaskBoardProps = PropsLocale<'teamCenter'> & {
    readonly phase: LoadPhase;
    readonly tasks: readonly TaskEntry[];
    readonly source: TeamSource;
    readonly notify: (text: string, level: 'info' | 'error') => void;
};
/**
 * 渲染任务板：行内状态+负责人+阻塞来源；操作走真实 RPC，审查走风险确认。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 任务板。
 */
export declare function TaskBoard({ t, phase, tasks, source, notify }: TaskBoardProps): import("react").JSX.Element;
//# sourceMappingURL=TaskBoard.d.ts.map
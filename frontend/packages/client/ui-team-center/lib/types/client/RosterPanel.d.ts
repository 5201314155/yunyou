import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { LoadPhase, RosterEntry, TeamSource } from './contract.ts';
/** 名册栏 props（纯数据+回调，见五份额纪律）。 */
export type RosterPanelProps = PropsLocale<'teamCenter'> & {
    readonly phase: LoadPhase;
    readonly roster: readonly RosterEntry[];
    readonly onSpawn: () => void;
    readonly source: TeamSource;
    readonly notify: (text: string, level: 'info' | 'error') => void;
};
/**
 * 渲染名册栏：每行状态点+名字+当前任务；行内操作触发 L2/L3 确认流。
 * @param props - 快照、阶段、回调与本地化。
 * @returns 名册栏。
 */
export declare function RosterPanel({ t, phase, roster, onSpawn, source, notify }: RosterPanelProps): import("react").JSX.Element;
//# sourceMappingURL=RosterPanel.d.ts.map
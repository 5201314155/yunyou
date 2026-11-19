import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { TeamSource } from './contract.ts';
/** 拉人对话框 props。 */
export interface SpawnDialogProps {
    readonly t: PropsLocale<'teamCenter'>['t'];
    readonly open: boolean;
    readonly onClose: () => void;
    readonly source: TeamSource;
    readonly notify: (text: string, level: 'info' | 'error') => void;
}
/**
 * 渲染拉起成员表单：name/description/prompt/context 四字段；fork 附降级说明。
 * 提交走真实 spawn RPC（服务端重名 fail-loud 原样展示）。
 * @param props - 本地化、开合与远程源。
 * @returns 拉人对话框。
 */
export declare function SpawnDialog({ t, open, onClose, source, notify }: SpawnDialogProps): import("react").JSX.Element | null;
//# sourceMappingURL=SpawnDialog.d.ts.map
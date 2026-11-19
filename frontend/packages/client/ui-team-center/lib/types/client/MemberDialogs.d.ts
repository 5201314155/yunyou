import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { TeamCenterKey } from './locales.ts';
/** 对话框负荷：kick 走风险确认，message 走表单。 */
export type MemberDialog = {
    readonly kind: 'kick';
    readonly name: string;
} | {
    readonly kind: 'message';
    readonly name: string;
} | undefined;
/** 对话框组 props。 */
export interface MemberDialogsProps {
    readonly t: PropsLocale<'teamCenter'>['t'];
    readonly dialog: MemberDialog;
    readonly onClose: () => void;
    readonly onKick: (name: string) => Promise<void>;
    readonly onMessage: (name: string, body: string) => Promise<void>;
}
/**
 * 渲染当前成员对话框；同一时刻至多一个。踢人的主操作在勾选确认前置不可用。
 * @param props - 本地化、当前对话框与回调。
 * @returns 对话框组（无对话框时渲染 null）。
 */
export declare function MemberDialogs({ t, dialog, onClose, onKick, onMessage }: MemberDialogsProps): import("react").JSX.Element | null;
export type { TeamCenterKey };
//# sourceMappingURL=MemberDialogs.d.ts.map
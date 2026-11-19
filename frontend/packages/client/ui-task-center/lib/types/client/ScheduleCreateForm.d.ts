import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { TaskCenterSource } from './contract.ts';
/** 创建表单 props。 */
export interface ScheduleCreateFormProps {
    readonly t: PropsLocale<'taskCenter'>['t'];
    readonly open: boolean;
    readonly onClose: () => void;
    readonly source: TaskCenterSource;
    /** 创建成功后刷新列表（回显确认已含 nextFireMs）。 */
    readonly onCreated: () => void;
}
/**
 * 渲染创建表单：单次/间隔/cron 三选一；提交即回显 nextFireMs（响应即确认步，
 * 服务端 nowMs+nextFireMs 为准；durable 无跨重启开关）。
 * @param props - 本地化、开合与远程源。
 * @returns 创建表单。
 */
export declare function ScheduleCreateForm({ t, open, onClose, source, onCreated }: ScheduleCreateFormProps): import("react").JSX.Element | null;
//# sourceMappingURL=ScheduleCreateForm.d.ts.map
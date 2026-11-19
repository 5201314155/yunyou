/** 任务中心插件，浏览器半：左栏「任务」面板 + main 装配。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis';
import { type TaskCenterKey } from './locales.ts';
declare module '@deepseek-ai/dsh-client-ui-slots' {
    interface LocaleNamespaceMap {
        /** 任务中心文案。 */
        'taskCenter': TaskCenterKey;
    }
}
/** 任务中心面板 id（侧栏入口与 main 面板键同源）。 */
export declare const PANEL_ID = "task-center";
/** 必需服务：槽位与字典。 */
export declare const inject: string[];
/**
 * 任务中心装配：字典、面板 store、main 页面与侧栏入口。
 * @param ctx - 客户端根上下文。
 */
export declare function apply(ctx: ClientContext): void;
//# sourceMappingURL=index.d.ts.map
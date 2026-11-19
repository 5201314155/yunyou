/** 插件广场插件，浏览器半：左栏「插件」面板 + main 装配（阶段 1 骨架+诚实态）。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis';
import { type PluginCenterKey } from './locales.ts';
declare module '@deepseek-ai/dsh-client-ui-slots' {
    interface LocaleNamespaceMap {
        /** 插件广场文案。 */
        'pluginCenter': PluginCenterKey;
    }
}
/** 插件广场面板 id（侧栏入口与 main 面板键同源）。 */
export declare const PANEL_ID = "plugin-center";
/** 必需服务：槽位与字典。 */
export declare const inject: string[];
/**
 * 插件广场装配：字典、面板 store、main 页面与侧栏入口。
 * @param ctx - 客户端根上下文。
 */
export declare function apply(ctx: ClientContext): void;
//# sourceMappingURL=index.d.ts.map
/** 团队中心插件，浏览器半：左栏「团队」面板 + main 装配。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis';
import { type TeamCenterKey } from './locales.ts';
declare module '@deepseek-ai/dsh-client-ui-slots' {
    interface LocaleNamespaceMap {
        /** 团队中心文案。 */
        'teamCenter': TeamCenterKey;
    }
}
/** 团队中心面板 id（侧栏入口与 main 面板键同源）。 */
export declare const PANEL_ID = "team-center";
/** 必需服务：槽位与字典。 */
export declare const inject: string[];
/**
 * 团队中心装配：字典、面板 store、main 页面与侧栏入口。
 * 数据拉取编排住在页面组件（props.source+props.actions）；此处只声明注册面。
 * @param ctx - 客户端根上下文。
 */
export declare function apply(ctx: ClientContext): void;
//# sourceMappingURL=index.d.ts.map
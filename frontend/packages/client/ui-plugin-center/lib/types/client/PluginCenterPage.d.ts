import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots';
/** 宿主注入面：远程源。 */
export interface PluginCenterInjected {
    readonly source: import('./contract.ts').MarketSource;
}
/** 广场页 props（五份额派生）。 */
export type PluginCenterPageProps = PropsRuntime<'main'> & InjectFace<PluginCenterInjected> & PropsStore<import('./stores.ts').MarketStoreHandle> & PropsLocale<'pluginCenter'>;
/**
 * 渲染插件广场：搜索+分类过滤+卡片网格；选中卡片展开三步详情。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 插件广场页。
 */
export declare function PluginCenterPage(props: PluginCenterPageProps): import("react").JSX.Element;
//# sourceMappingURL=PluginCenterPage.d.ts.map
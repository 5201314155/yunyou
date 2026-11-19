import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { MarketEntry } from './contract.ts';
/** 详情 props。 */
export interface PluginDetailProps {
    readonly t: PropsLocale<'pluginCenter'>['t'];
    readonly entry: MarketEntry;
    readonly source: import('./contract.ts').MarketSource;
    readonly onBack: () => void;
}
/**
 * 渲染详情：拉取三步文档序步骤并按序呈现；失败保留返回路径（fail-visible）。
 * @param props - 条目、远程源与本地化。
 * @returns 详情面板。
 */
export declare function PluginDetail({ t, entry, source, onBack }: PluginDetailProps): import("react").JSX.Element;
//# sourceMappingURL=PluginDetail.d.ts.map
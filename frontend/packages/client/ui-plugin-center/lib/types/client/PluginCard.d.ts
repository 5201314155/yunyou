import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots';
import type { MarketEntry } from './contract.ts';
/** 卡片 props。 */
export interface PluginCardProps {
    readonly t: PropsLocale<'pluginCenter'>['t'];
    readonly entry: MarketEntry;
    readonly onSelect: () => void;
}
/**
 * 渲染广场卡片；版本兼容不符或无 Web 面时灰置并给因（不可点装）。
 * @param props - 条目与本地化。
 * @returns 广场卡片。
 */
export declare function PluginCard({ t, entry, onSelect }: PluginCardProps): import("react").JSX.Element;
//# sourceMappingURL=PluginCard.d.ts.map
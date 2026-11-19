import { createMarketStore } from "./stores.js";
import { stage1MarketSource } from "./source.js";
import { PluginCenterPage } from "./PluginCenterPage.js";
import { PluginCenterIcon } from "./PluginCenterIcon.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/** 插件广场面板 id（侧栏入口与 main 面板键同源）。 */
export const PANEL_ID = 'plugin-center';
/** 必需服务：槽位与字典。 */
export const inject = ['slots', 'locale'];
/**
 * 插件广场装配：字典、面板 store、main 页面与侧栏入口。
 * @param ctx - 客户端根上下文。
 */
export function apply(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-plugin-center: dictionaries');
    const store = createMarketStore();
    const t = ctx.locale.bind(NS);
    ctx.slots.inject('main', () => ctx.slots.register({
        name: 'main',
        key: PANEL_ID,
        locale: NS,
        store,
        inject: () => ({ source: stage1MarketSource }),
    }, PluginCenterPage));
    ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
        name: 'sidebar.panellist',
        id: PANEL_ID,
        order: 14,
        locale: NS,
        label: () => t('panel.label'),
    }, PluginCenterIcon));
}
//# sourceMappingURL=index.js.map
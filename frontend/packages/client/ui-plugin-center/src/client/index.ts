/** 插件广场插件，浏览器半：左栏「插件」面板 + main 装配（阶段 1 骨架+诚实态）。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-client-locale/client'
import type {} from '@deepseek-ai/dsh-client-ui-renderer/client'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'
import { createMarketStore } from './stores.ts'
import { stage1MarketSource } from './source.ts'
import { PluginCenterPage } from './PluginCenterPage.tsx'
import { PluginCenterIcon } from './PluginCenterIcon.tsx'
import { en, ja, es, fr, de, ru, pt, NS, zh, type PluginCenterKey } from './locales.ts'

declare module '@deepseek-ai/dsh-client-ui-slots' {
  interface LocaleNamespaceMap {
    /** 插件广场文案。 */
    'pluginCenter': PluginCenterKey
  }
}

/** 插件广场面板 id（侧栏入口与 main 面板键同源）。 */
export const PANEL_ID = 'plugin-center'

/** 必需服务：槽位与字典。 */
export const inject = ['slots', 'locale']

/**
 * 插件广场装配：字典、面板 store、main 页面与侧栏入口。
 * @param ctx - 客户端根上下文。
 */
export function apply(ctx: ClientContext): void {
  ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-plugin-center: dictionaries')
  const store = createMarketStore()
  const t = ctx.locale.bind(NS)
  ctx.slots.inject('main', () => ctx.slots.register({
    name: 'main',
    key: PANEL_ID,
    locale: NS,
    store,
    inject: () => ({ source: stage1MarketSource }),
  }, PluginCenterPage))
  ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
    name: 'sidebar.panellist',
    id: PANEL_ID,
    order: 14,
    locale: NS,
    label: () => t('panel.label'),
  }, PluginCenterIcon))
}

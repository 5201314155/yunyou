import { createTeamStore } from "./stores.js";
import { createTeamSource } from "./source.js";
import { TeamCenterPage } from "./TeamCenterPage.js";
import { TeamCenterIcon } from "./TeamCenterIcon.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/** 团队中心面板 id（侧栏入口与 main 面板键同源）。 */
export const PANEL_ID = 'team-center';
/** 必需服务：槽位与字典。 */
export const inject = ['slots', 'locale'];
/**
 * 团队中心装配：字典、面板 store、main 页面与侧栏入口。
 * 数据拉取编排住在页面组件（props.source+props.actions）；此处只声明注册面。
 * @param ctx - 客户端根上下文。
 */
export function apply(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-team-center: dictionaries');
    const store = createTeamStore();
    const t = ctx.locale.bind(NS);
    ctx.slots.inject('main', () => ctx.slots.register({
        name: 'main',
        key: PANEL_ID,
        locale: NS,
        store,
        inject: () => ({ source: createTeamSource(ctx) }),
    }, TeamCenterPage));
    ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
        name: 'sidebar.panellist',
        id: PANEL_ID,
        order: 12,
        locale: NS,
        label: () => t('panel.label'),
    }, TeamCenterIcon));
}
//# sourceMappingURL=index.js.map
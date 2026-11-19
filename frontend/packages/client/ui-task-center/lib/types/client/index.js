import { createTaskCenterStore } from "./stores.js";
import { createTaskCenterSource } from "./source.js";
import { TaskCenterPage } from "./TaskCenterPage.js";
import { TaskCenterIcon } from "./TaskCenterIcon.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/** 任务中心面板 id（侧栏入口与 main 面板键同源）。 */
export const PANEL_ID = 'task-center';
/** 必需服务：槽位与字典。 */
export const inject = ['slots', 'locale'];
/**
 * 任务中心装配：字典、面板 store、main 页面与侧栏入口。
 * @param ctx - 客户端根上下文。
 */
export function apply(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-task-center: dictionaries');
    const store = createTaskCenterStore();
    const t = ctx.locale.bind(NS);
    ctx.slots.inject('main', () => ctx.slots.register({
        name: 'main',
        key: PANEL_ID,
        locale: NS,
        store,
        inject: () => ({ source: createTaskCenterSource(ctx) }),
    }, TaskCenterPage));
    ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
        name: 'sidebar.panellist',
        id: PANEL_ID,
        order: 13,
        locale: NS,
        label: () => t('panel.label'),
    }, TaskCenterIcon));
}
//# sourceMappingURL=index.js.map
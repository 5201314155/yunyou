import { SubagentCatalogAction, SubagentHeaderLineage } from "./SubagentHeaderLineage.js";
import { SubagentReadOnlyComposer, } from "./SubagentReadOnlyComposer.js";
import { registerSidebarChat, subagentChatAddress } from "./sidebar-chat/index.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/** Required services for subagent presentation and navigation. */
export const inject = ['sessions', 'uiWorkspace', 'slots', 'locale', 'sidebarRight'];
/** Claim the composer when the child mode does not allow follow-ups. */
function selectReadOnlySubagent(owner) {
    const subagent = owner.session?.subagent;
    if (subagent === undefined || subagent === null)
        return null;
    if (subagent.address.mode === 'unknown')
        return { reason: 'unknown' };
    if (subagent.address.mode === 'one-shot')
        return { reason: 'one-shot' };
    return null;
}
/**
 * Client plugin body: register the subagent catalog and read-only composer seats.
 * @param ctx - client root context.
 */
export function apply(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'ui-subagent: dictionaries');
    ctx.inject(['resources', 'sidebarRightTabs'], (scope) => {
        registerSidebarChat(scope, ctx.locale.bind(NS));
    });
    const catalogActions = (_parentSessionId) => ({
        openChild(address) {
            ctx.uiWorkspace.openSession(address);
        },
        openChildAside(address) {
            ctx.sidebarRight.openResource(subagentChatAddress(address), {
                kind: 'subagentchat',
                preferNewPane: true,
            });
        },
        async prepareCatalog(parentSessionId) {
            await ctx.sessions.refreshProjections(parentSessionId);
            const parent = ctx.sessions.list.getSnapshot().projectionsBySession[parentSessionId];
            if (parent?.state !== 'ready')
                return;
            for (const entry of parent.values.subagentCatalog ?? []) {
                if (entry.mode === 'external')
                    continue;
                const state = ctx.sessions.list.getSnapshot();
                if (state.byId[entry.id]?.formatStatus === 'migration-required'
                    || state.projectionsBySession[entry.id]?.state === 'migration-required'
                    || state.projectionsBySession[entry.id]?.state === 'error')
                    continue;
                await ctx.sessions.refreshProjections(entry.id);
            }
        },
        refreshProjection(parentSessionId) {
            void ctx.sessions.refreshProjections(parentSessionId);
        },
    });
    const footerAvailable = {
        getSnapshot: () => ctx.slots.entries('conversation.session.header.actions.subagent.footer').length > 0,
        subscribe: (listener) => ctx.slots.subscribe('conversation.session.header.actions.subagent.footer', listener),
    };
    ctx.slots.inject('conversation.session.header.lineage', () => ctx.slots.register({
        name: 'conversation.session.header.lineage',
        locale: NS,
        inject: catalogActions,
    }, SubagentHeaderLineage));
    ctx.slots.inject('conversation.session.header.actions', () => ctx.slots.register({
        name: 'conversation.session.header.actions',
        id: 'subagent-catalog',
        // Between the preset label (-10) and background tasks (20).
        order: 10,
        locale: NS,
        inject: (sessionId) => ({
            ...catalogActions(sessionId), hooks: { footerAvailable },
        }),
        children: { 'conversation.session.header.actions.subagent.footer': { kind: 'list', scope: 'session' } },
    }, SubagentCatalogAction));
    ctx.slots.inject('conversation.composer', () => ctx.slots.register({
        name: 'conversation.composer',
        priority: -10,
        locale: NS,
        select: selectReadOnlySubagent,
    }, SubagentReadOnlyComposer));
}
//# sourceMappingURL=index.js.map
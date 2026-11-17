/**
 * Static access for client specs. Production pages receive the
 * `remoteAccess` Client service and the global-resource `useRemoteAccess` Hook
 * from the remote-access Client plugin, and the Session and Workspace access
 * Hooks (Session-scope `useAccess`, `useSessionAccess`, `useWorkspaceAccess`)
 * from ui-session and ui-workspace over that service. The test runtimes
 * install an allow-all service and `useRemoteAccess` Hook instead, and
 * hand-built component props spread {@link allowAllAccess}; a spec of a
 * refused path overrides one Hook or provides a {@link StaticRemoteAccess}
 * that refuses the action.
 * @module @deepseek-ai/dsh-client-test-runtime/src/access
 */
import { Service } from '@deepseek-ai/cordis';
/** Access Hooks that allow every action on every resource, for hand-built component props. */
export const allowAllAccess = {
    useRemoteAccess: () => true,
    useAccess: () => true,
    useSessionAccess: () => true,
    useWorkspaceAccess: () => true,
};
const ALLOWED = { getSnapshot: () => true, subscribe: () => () => { } };
const REFUSED = { getSnapshot: () => false, subscribe: () => () => { } };
/**
 * A `remoteAccess` Client service whose answers never change: `allows`
 * decides each action on every resource once. `whenAccess` installs an
 * allowed action's effects in the caller's fiber at once and never installs a
 * refused action's effects.
 */
export class StaticRemoteAccess extends Service {
    allows;
    /**
     * @param ctx - Context the service is provided on.
     * @param allows - the fixed answer for each action; allows everything when omitted.
     */
    constructor(ctx, allows = () => true) {
        super(ctx, 'remoteAccess');
        this.allows = allows;
    }
    source(_kind, _id, action) {
        return this.allows(action) ? ALLOWED : REFUSED;
    }
    // Prototype methods: the Cordis service proxy binds `this.ctx` to the caller's Context.
    whenAccess(action, callback) {
        if (!this.allows(action))
            return () => { };
        const dispose = this.ctx.effect(callback, `remoteAccess.whenAccess(${JSON.stringify(action)})`);
        return () => { void dispose(); };
    }
    whenResourceAccess(_kind, _id, action, callback) {
        return this.whenAccess(action, callback);
    }
}
/**
 * Plugin standing in for the remote-access Client row with fixed answers: it
 * provides a {@link StaticRemoteAccess}, so ui-session and ui-workspace
 * publish their resource Hooks over it, and the matching `useRemoteAccess`
 * Hook on the global resource.
 * @param allows - the fixed answer for each action on every resource.
 * @returns the plugin.
 */
export function staticAccessPlugin(allows) {
    return {
        name: 'client-test-runtime:static-access',
        inject: ['slots'],
        apply(ctx) {
            new StaticRemoteAccess(ctx, allows);
            ctx.slots.provideRoot({ keyedHooks: { remoteAccess: (action) => (allows(action) ? ALLOWED : REFUSED) } });
        },
    };
}
/** {@link staticAccessPlugin} allowing every action on every resource. */
export const allowAllAccessPlugin = staticAccessPlugin(() => true);
//# sourceMappingURL=access.js.map
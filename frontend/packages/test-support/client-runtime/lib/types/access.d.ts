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
import { Service, type Context } from '@deepseek-ai/cordis';
import type { AccessEffect, ClientRemoteAccess, RemoteEndpoint, TypertResourceKind } from '@deepseek-ai/dsh-api-remote-access/client';
import type { GlobalStandardProps, HostObservable, SessionStandardProps } from '@deepseek-ai/dsh-client-ui-slots';
/** The access Hooks every Slot component receives. */
export type AccessStandardProps = Pick<GlobalStandardProps, 'useRemoteAccess' | 'useSessionAccess' | 'useWorkspaceAccess'> & Pick<SessionStandardProps, 'useAccess'>;
/** Access Hooks that allow every action on every resource, for hand-built component props. */
export declare const allowAllAccess: AccessStandardProps;
/**
 * A `remoteAccess` Client service whose answers never change: `allows`
 * decides each action on every resource once. `whenAccess` installs an
 * allowed action's effects in the caller's fiber at once and never installs a
 * refused action's effects.
 */
export declare class StaticRemoteAccess extends Service implements ClientRemoteAccess {
    private readonly allows;
    /**
     * @param ctx - Context the service is provided on.
     * @param allows - the fixed answer for each action; allows everything when omitted.
     */
    constructor(ctx: Context, allows?: (action: string) => boolean);
    source(_kind: TypertResourceKind, _id: string, action: string): HostObservable<boolean>;
    whenAccess(action: RemoteEndpoint, callback: () => AccessEffect): () => void;
    whenResourceAccess(_kind: TypertResourceKind, _id: string, action: RemoteEndpoint, callback: () => AccessEffect): () => void;
}
/** The plugin shape {@link staticAccessPlugin} returns. */
export interface StaticAccessPlugin {
    readonly name: string;
    readonly inject: readonly string[];
    apply(ctx: Context): void;
}
/**
 * Plugin standing in for the remote-access Client row with fixed answers: it
 * provides a {@link StaticRemoteAccess}, so ui-session and ui-workspace
 * publish their resource Hooks over it, and the matching `useRemoteAccess`
 * Hook on the global resource.
 * @param allows - the fixed answer for each action on every resource.
 * @returns the plugin.
 */
export declare function staticAccessPlugin(allows: (action: string) => boolean): StaticAccessPlugin;
/** {@link staticAccessPlugin} allowing every action on every resource. */
export declare const allowAllAccessPlugin: StaticAccessPlugin;
//# sourceMappingURL=access.d.ts.map
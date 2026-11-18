/** Session-addressed, cold-readable skill catalog Remote. */
import type { Context } from '@deepseek-ai/cordis';
import { TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol';
import type { SessionSkillGetRequest, SessionSkillGetValue, SkillListRequest, SkillListValue } from './types.ts';
declare module '@deepseek-ai/cordis' {
    interface Context {
        /** Host owner of the Session-addressed `skills` Remote namespace. */
        sessionSkillCatalog: SessionSkillCatalog;
    }
}
/** Host service backing `ctx.remote.skills` without activating a cold Agent. */
export declare class SessionSkillCatalog extends TypertRemoteService {
    static inject: string[];
    /** @param ctx - Host context carrying Session reads and optional skill/preset services. */
    constructor(ctx: Context);
    /**
     * List the user-invocable skills visible to one Session composition.
     * @param request - Session identity whose cwd and preset select the catalog view.
     * @param signal - caller lifetime carried by the Remote transport; admitted catalog reads retain their existing completion semantics.
     * @returns user-invocable skill metadata without loading skill bodies.
     * @throws RemoteError when the Session cannot be inspected or no registry can serve it.
     */
    list(request: SkillListRequest, signal: AbortSignal): Promise<SkillListValue>;
    /**
     * Read one user-invocable skill's full body for human viewing and explicit
     * fill-back (web.v1 §9.2: a skill is a read-only catalog entry, never an
     * executable RPC).
     * @param request - Session identity selecting the composition and the skill id.
     * @returns the skill metadata and its raw body text.
     * @throws RemoteError when the skill is absent from the Session's composition.
     */
    get(request: SessionSkillGetRequest): Promise<SessionSkillGetValue>;
    /** Resolve a live or standing preset scope without creating an Agent. */
    private scopeFor;
    /** Resolve one Session's cwd, viewing scope, and skill registry. */
    private compositionView;
    /** The user-invocable summaries of one composition. */
    private visibleSkills;
}
export default SessionSkillCatalog;
//# sourceMappingURL=skill-catalog.d.ts.map
import { PatchParseError } from '@deepseek-ai/dsh-fs-patch';
import { commitPatch } from "./execute.js";
import { planPatch } from "./plan.js";
import { preflightPatch } from "./preflight.js";
/**
 * Preflight a whole patch before sequential guarded mutations; business failures become records.
 * @param ctx - mounted filesystem and observation-policy events.
 * @param sandbox - denial mapper shared with existing filesystem tools.
 * @param exec - session cwd, observation actor, and cancellation signal.
 * @param request - raw patch and policy already resolved by the caller.
 * @returns the syntax failure, the preflight refusals, or ordered commit outcomes.
 * @throws unexpected programming defects without starting further operations.
 */
export async function executePatch(ctx, sandbox, exec, request) {
    let plan;
    try {
        plan = await planPatch(ctx, sandbox, exec, request);
    }
    catch (error) {
        if (!(error instanceof PatchParseError))
            throw error;
        return { phase: 'syntax', failure: { code: 'PATCH_SYNTAX', message: error.message } };
    }
    const [first, ...rest] = [
        ...plan.refusals,
        ...await preflightPatch(ctx, sandbox, exec, request.sandboxPolicy, plan.operations),
    ].sort((left, right) => left.operation.index - right.operation.index);
    if (first !== undefined)
        return { phase: 'preflight', refusals: [first, ...rest] };
    return commitPatch(ctx, sandbox, exec, request.sandboxPolicy, plan.operations);
}
//# sourceMappingURL=index.js.map
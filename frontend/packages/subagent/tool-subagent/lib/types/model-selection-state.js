/** Durable per-session state for the user-controlled model-selection opt-in. */
import { z as zod } from 'zod';
import { assertAllowedModelRoutes } from "./model-selection.js";
const modelSelectionPolicySchema = zod.array(zod.object({
    provider: zod.string().min(1),
    model: zod.string().min(1),
}).strict()).min(1).nullable();
/**
 * Host-only projection of the durable model-selection policy.
 * Each independently mounted consumer registers this shared definition; the registry reference-counts it.
 */
export const subagentModelSelectionProjectionDefinition = {
    key: 'subagentModelSelectionPolicy',
    stateVersion: 1,
    stateSchema: modelSelectionPolicySchema,
    init: () => null,
    apply: (policy, event) => {
        if (policy !== null || event.type !== 'subagent/model-selection-policy')
            return policy;
        const { allowedModels } = event.data;
        assertAllowedModelRoutes(allowedModels);
        if (allowedModels.length === 0) {
            throw new Error('subagent/model-selection-policy requires at least one route');
        }
        return allowedModels;
    },
};
/**
 * Read the exact route list authorized for a Session.
 * @param subagentModelSelectionProjectionHandle - registered policy state reader.
 * @param session - session whose durable decision is read.
 * @returns a detached route list, or undefined to restrict selection to the parent route.
 */
export function subagentModelSelectionPolicy(subagentModelSelectionProjectionHandle, session) {
    return subagentModelSelectionProjectionHandle.stateOf(session)?.map(route => ({ ...route }));
}
//# sourceMappingURL=model-selection-state.js.map
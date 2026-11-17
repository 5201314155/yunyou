import { Service } from '@deepseek-ai/cordis';
import z from '@deepseek-ai/schemastery';
import { SessionSeq } from '@deepseek-ai/dsh-session';
import { subagentModelSelectionPolicy, subagentModelSelectionProjectionDefinition, } from "./model-selection-state.js";
import { AllowedModelRouteSchema, assertAllowedModelRoutes, } from "./model-selection.js";
/** Host settings owner that records Session authorization before the first model request. */
export class SubagentModelSelectionConfig extends Service {
    config;
    static inject = ['sessionProjections', 'sessions'];
    static Config = z.object({
        enabled: z.boolean().default(false).volatile(),
        allowedModels: z.array(AllowedModelRouteSchema).default([]).volatile(),
    });
    constructor(ctx, config) {
        super(ctx, 'subagentModelSelection');
        this.config = config;
    }
    async [Service.init]() {
        const ctx = this.ctx;
        const policyReady = ctx.sessionProjections.register(subagentModelSelectionProjectionDefinition);
        ctx.on('agent/created', async ({ agent }) => {
            const policy = await policyReady;
            ctx.fiber.assertActive();
            const target = agent.session;
            if (subagentModelSelectionPolicy(policy, target) !== undefined)
                return;
            let allowedModels;
            const parentId = target.header.origin === 'subagent' ? target.header.parentSession : undefined;
            if (parentId !== undefined) {
                const parent = ctx.sessions.get(parentId);
                if (parent !== undefined)
                    allowedModels = subagentModelSelectionPolicy(policy, parent);
            }
            else if (target.firstLiveSeq === 0
                // oxlint-disable-next-line typescript/no-deprecated -- Existing Session history read; migration deferred.
                && target.eventAt(SessionSeq(0))?.type !== 'session/end-seed') {
                const current = this.current();
                allowedModels = current.enabled ? current.allowedModels : undefined;
            }
            if (allowedModels !== undefined)
                target.append('subagent/model-selection-policy', { allowedModels });
        });
        await policyReady;
    }
    /**
     * Read a detached selection preference for the next fresh top-level Session.
     * @returns the enabled state and exact allowed routes.
     */
    current() {
        const enabled = this.config.enabled.get();
        const allowedModels = this.config.allowedModels.get();
        assertAllowedModelRoutes(allowedModels);
        if (enabled && allowedModels.length === 0) {
            throw new Error('enabled subagent model selection requires at least one allowed model');
        }
        return { enabled, allowedModels: allowedModels.map(route => ({ ...route })) };
    }
}
export const name = 'subagent-model-selection-settings';
export default SubagentModelSelectionConfig;
//# sourceMappingURL=model-selection-settings.js.map
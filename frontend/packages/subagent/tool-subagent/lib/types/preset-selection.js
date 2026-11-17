/** Validates configured choices and resolves model input against the allowlist. */
export class SubagentPresetSelection {
    config;
    constructor(config) {
        this.config = config;
        const { agentPreset, allowedAgentPresets } = config;
        if (allowedAgentPresets === undefined)
            return;
        if (new Set(allowedAgentPresets).size !== allowedAgentPresets.length) {
            throw new Error('tool-subagent: allowedAgentPresets contains duplicate preset ids');
        }
        if (agentPreset !== undefined && !allowedAgentPresets.includes(agentPreset)) {
            throw new Error('tool-subagent: agentPreset must be included in allowedAgentPresets');
        }
    }
    /** Whether this tool requires the preset service and an independently composing provider. */
    get configured() {
        return this.config.agentPreset !== undefined || this.config.allowedAgentPresets !== undefined;
    }
    /** Preset ids advertised in the model parameter. */
    get choices() {
        return this.config.allowedAgentPresets;
    }
    /** Model-visible selection and default guidance. */
    get description() {
        const fallback = this.config.agentPreset === undefined
            ? 'Omit to inherit the parent agent\'s current preset.'
            : `Omit to use ${this.config.agentPreset}.`;
        return `Agent preset for this child. ${fallback}`;
    }
    /**
     * Reject independent selection for providers that inherit conversation history or lack support.
     * @param provider - Currently registered delegation provider.
     */
    assertProvider(provider) {
        if (this.configured && (!provider.capabilities.agentPreset || provider.inheritsParentContext)) {
            throw new Error(`tool-subagent: provider "${provider.name}" does not support independent agent preset selection`);
        }
    }
    /**
     * Validate model input and apply the deployment default.
     * @param request - Model-supplied tool arguments.
     * @returns Selected preset id, or omission to inherit the parent.
     */
    resolve(request) {
        const id = request.agent_preset;
        if (id === undefined)
            return this.config.agentPreset;
        if (this.choices === undefined)
            throw new Error('agent_preset selection is disabled for this tool instance');
        if (typeof id !== 'string' || !this.choices.includes(id)) {
            throw new Error(`agent_preset must be one of: ${this.choices.join(', ')}`);
        }
        return id;
    }
}
//# sourceMappingURL=preset-selection.js.map
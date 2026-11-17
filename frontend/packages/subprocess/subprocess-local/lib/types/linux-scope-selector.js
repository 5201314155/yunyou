/** Linux native containment decisions over injected host observations. */
/**
 * Provider-lifetime Linux scope selection. A probe runs on the first selection and again only after
 * a scope launch never started its target, a different user-manager listener appears under a
 * negative verdict, or the systemd-run binary changes under a positive one. A positive verdict
 * confirms the user manager on every selection. Selection is synchronous, so a decision stays
 * valid for the launch that follows it.
 */
export class LinuxScopeSelector {
    observe;
    state = { kind: 'unprobed', trigger: 'first' };
    generation = 0;
    /** @param observe - host checks consulted by each selection. */
    constructor(observe) {
        this.observe = observe;
    }
    /**
     * Choose containment for one eligible launch.
     * @returns the decision and the branch that produced it.
     */
    select() {
        const state = this.state;
        if (state.kind === 'unprobed')
            return this.probe(state.trigger);
        if (state.kind === 'available') {
            if (this.observe.launcherIdentity() !== state.launcher)
                return this.probe('launcher-changed');
            return this.observe.confirmManager()
                ? { kind: 'scope', reason: 'manager-confirmed', literalArgv: state.literalArgv, generation: this.generation, probe: undefined }
                : { kind: 'fallback', reason: 'manager-unconfirmed', probe: undefined };
        }
        const listener = this.observe.managerListener();
        if (listener === undefined)
            return { kind: 'fallback', reason: 'no-listener', probe: undefined };
        if (listener === state.listener)
            return { kind: 'fallback', reason: 'same-listener', probe: undefined };
        return this.probe('new-listener');
    }
    /**
     * Record that a scope launched under generation exited before its bootstrap started the target,
     * so the next selection probes again. Launches from a superseded probe are ignored.
     * @param generation - generation of the scope decision that launched it.
     */
    scopeNotStarted(generation) {
        if (generation === this.generation)
            this.state = { kind: 'unprobed', trigger: 'scope-not-started' };
    }
    probe(trigger) {
        this.generation += 1;
        // Read before probing: a manager that starts during a failed probe then counts as a new listener.
        const listener = this.observe.managerListener();
        const launcher = this.observe.launcherIdentity();
        const literalArgv = this.observe.probe();
        if (literalArgv === undefined) {
            this.state = { kind: 'unavailable', listener };
            return { kind: 'fallback', reason: 'probe-unavailable', probe: trigger };
        }
        this.state = { kind: 'available', literalArgv, launcher };
        return { kind: 'scope', reason: 'probe-available', literalArgv, generation: this.generation, probe: trigger };
    }
}
//# sourceMappingURL=linux-scope-selector.js.map
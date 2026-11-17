/** Source-safe task-board browser registration. */
import { createSnapshotStore } from '@deepseek-ai/dsh-client-store';
import { TeamDisableConfirmation } from "./TeamDisableConfirmation.js";
import { TeamModeToggle } from "./TeamModeToggle.js";
import { TeamBoard } from "./TeamBoard.js";
import { TeamCommandRow } from "./TeamCommandRow.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
/** Required browser services for slots and localized copy. */
export const inject = ['slots', 'locale'];
/**
 * Register the task-board dictionaries and subagent catalog sections.
 * The panel reads the root Session’s `agentTeam` projection through standard Session hooks.
 * @param ctx - Client Context carrying the injected locale and slot services.
 */
export function registerAgentTeamUi(ctx) {
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'client-ui-agent-team: dictionaries');
    ctx.slots.inject('conversation.chat.commandview', () => ctx.slots.register({
        name: 'conversation.chat.commandview', key: 'team', locale: NS,
    }, TeamCommandRow));
    ctx.inject(['remote', 'remote.agentTeams'], (scope) => {
        scope.slots.inject('shell.overlay', function* () {
            const disableRequest = createSnapshotStore(null);
            const selectTeamMode = async (sessionId, enabled) => {
                const response = await scope.remote.agentTeams.setEnabled(sessionId, enabled);
                return response.ok ? null : response.error.message;
            };
            yield () => { disableRequest.set(null); };
            yield scope.slots.register({
                name: 'shell.overlay', id: 'agent-team.disable', locale: NS,
                inject: () => ({
                    hooks: { disableRequest },
                    cancelDisable: (sessionId) => {
                        if (disableRequest.getSnapshot()?.sessionId === sessionId)
                            disableRequest.set(null);
                    },
                    disableTeam: sessionId => selectTeamMode(sessionId, false),
                }),
            }, TeamDisableConfirmation);
            yield scope.slots.inject('conversation.input.model.options', () => scope.slots.register({
                name: 'conversation.input.model.options',
                id: 'agent-team',
                locale: NS,
                inject: (sessionId) => ({
                    selectTeamMode: enabled => selectTeamMode(sessionId, enabled),
                    requestDisableConfirmation: () => { disableRequest.set({ sessionId }); },
                }),
            }, TeamModeToggle));
        });
    });
    ctx.slots.inject('conversation.session.header.actions.subagent.footer', () => ctx.slots.register({
        name: 'conversation.session.header.actions.subagent.footer', id: 'agent-team', locale: NS,
    }, TeamBoard));
}
//# sourceMappingURL=mount.js.map
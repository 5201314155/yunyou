/** Browser entry registering the task-board sections in the subagent catalog. */
import { registerAgentTeamUi } from "./mount.js";
export { inject } from "./mount.js";
/**
 * Register the task-board locale dictionaries and catalog sections on the Client Context.
 * @param ctx - Client Context with the declared `inject` services available.
 */
export function apply(ctx) {
    registerAgentTeamUi(ctx);
}
//# sourceMappingURL=index.js.map
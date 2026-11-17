import { NOTIFICATIONS_CONFIG_GLOBAL } from "./config.js";
export { Config } from "./config.js";
/** @param ctx - Host page bootstrap collector. @param config - validated notification configuration. */
export function apply(ctx, config) {
    ctx.on('webserver/index-inject', (table) => {
        table.push({ kind: 'global', name: NOTIFICATIONS_CONFIG_GLOBAL, value: config });
    });
}
//# sourceMappingURL=index.js.map
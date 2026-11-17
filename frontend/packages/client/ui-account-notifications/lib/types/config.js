/** Public polling configuration shared by the notification Host and Client halves. */
import Schema from '@deepseek-ai/schemastery';
/** Validate notification polling configuration. */
export const Config = Schema.object({
    notificationPollIntervalMs: Schema.natural().min(1).max(2_147_483_647).default(600_000),
});
/** Bootstrap key for public notification configuration. */
export const NOTIFICATIONS_CONFIG_GLOBAL = '__DSH_ACCOUNT_NOTIFICATIONS_CONFIG__';
//# sourceMappingURL=config.js.map
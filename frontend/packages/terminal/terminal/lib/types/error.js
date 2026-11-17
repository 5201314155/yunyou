/** Machine-routable PTY service and backend failures. @module */
import { HarnessError } from '@deepseek-ai/dsh-error';
/** Harness error carrying a stable {@link TerminalErrorCode}. */
export class TerminalError extends HarnessError {
    code;
    /**
     * @param message - names the PTY operation and the safe next step.
     * @param code - the failure class consumers route on.
     * @param options - standard `cause` chaining.
     */
    constructor(message, code, options) {
        super(message, code, options);
        this.name = 'TerminalError';
        this.code = code;
    }
}
/**
 * Backend-reported failure to clean partial resources after unpublished setup
 * failed; code `CLEANUP_FAILED`. Backend-owned resources may still be alive.
 * `errors` lists the setup failure, then the cleanup failure; `cause` is an
 * `AggregateError` of the same two failures, so renderers that follow `cause`
 * report both.
 */
export class TerminalBackendCleanupError extends TerminalError {
    spawnError;
    cleanupError;
    /** The setup failure followed by the cleanup failure. */
    errors;
    /**
     * @param spawnError - original setup or cancellation failure.
     * @param cleanupError - failure that may leave backend-owned resources alive.
     */
    constructor(spawnError, cleanupError) {
        super('PTY backend startup and cleanup both failed', 'CLEANUP_FAILED', {
            cause: new AggregateError([spawnError, cleanupError], 'PTY backend startup and cleanup both failed'),
        });
        this.spawnError = spawnError;
        this.cleanupError = cleanupError;
        this.name = 'TerminalBackendCleanupError';
        this.errors = [spawnError, cleanupError];
    }
}
//# sourceMappingURL=error.js.map
/** Machine-routable shell environment failures. @module */
import { HarnessError } from '@deepseek-ai/dsh-error';
/**
 * Every {@link ShellEnvError} code:
 * - `SHELL_ENV_CONTRIBUTION_INVALID` — a registered contributor returned an undeclared key or a non-string value; the
 *   shell command does not run, and the contributing plugin must be fixed.
 */
export const SHELL_ENV_ERROR_CODES = ['SHELL_ENV_CONTRIBUTION_INVALID'];
/** Shell environment failure carrying a stable {@link ShellEnvErrorCode}. */
export class ShellEnvError extends HarnessError {
    code;
    /**
     * @param message - names the contributor and the offending key.
     * @param code - the failure class consumers route on.
     */
    constructor(message, code) {
        super(message, code);
        this.name = 'ShellEnvError';
        this.code = code;
    }
}
//# sourceMappingURL=error.js.map
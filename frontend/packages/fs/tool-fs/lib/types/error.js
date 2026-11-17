/**
 * Model-facing diagnostics for guarded-mutation failures. Providers and
 * policies retain operation-specific causes, while this package owns the
 * stable message shown to the model.
 * @module @deepseek-ai/dsh-tool-fs/src/error
 */
import { FsError } from '@deepseek-ai/dsh-fs';
export function remediateFsError(error, displayPath) {
    if (!(error instanceof FsError))
        return error;
    if (error.code === 'FS_NOT_OBSERVED') {
        return new FsError(`cannot modify "${displayPath}": file has not been read — read the file, then retry`, error.code, { cause: error });
    }
    if (error.code === 'FS_STALE_VERSION') {
        return new FsError(`${error.message} — re-read the file, then retry`, error.code, { cause: error });
    }
    return error;
}
//# sourceMappingURL=error.js.map
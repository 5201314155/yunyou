/**
 * `internal/modules/esm/loader` cascaded loader for the Worker. {@link createCascadedLoader} builds the object with
 * the Node v24.18.0 (v2) loader members this repository calls: `resolveSync` and `import` over one Worker module
 * loader, and an empty `loadCache`. ES module jobs are unavailable on this platform.
 * @module @deepseek-ai/dsh-experimental-webworker-runtime/src/node/internal_modules/esm-loader
 */
import { notAvailableError } from "../notImplementedFail.js";
import { LoadCache } from "./module-map.js";
/**
 * Translate a Worker resolution failure into the code Node's ESM resolver reports.
 * @param error - Failure thrown by the Worker module loader.
 * @returns The same failure with an ESM resolution code.
 */
function esmResolutionError(error) {
    if (error instanceof Error && error.code === 'MODULE_NOT_FOUND') {
        return Object.assign(error, { code: 'ERR_MODULE_NOT_FOUND' });
    }
    return error;
}
/**
 * Build the cascaded loader for one Worker module loader.
 * @param source - The Worker module loader's resolution and loading operations.
 * @returns A cascaded loader delegating to `source`.
 */
export function createCascadedLoader(source) {
    return {
        loadCache: new LoadCache(),
        resolveSync(parentURL, request) {
            try {
                const { url, format } = source.resolveSync(request.specifier, parentURL);
                return { url, format };
            }
            catch (error) {
                throw esmResolutionError(error);
            }
        },
        import(specifier, parentURL) {
            return source.import(specifier, parentURL);
        },
        getOrCreateModuleJob() {
            throw notAvailableError('internal/modules/esm/loader', 'getOrCreateModuleJob');
        },
    };
}
//# sourceMappingURL=esm-loader.js.map
/**
 * `internal/modules/esm/module_map` for the Worker, following Node v24.18.0. The Worker evaluates no ES module
 * jobs, so a {@link LoadCache} stays empty and refuses to store one.
 * @module @deepseek-ai/dsh-experimental-webworker-runtime/src/node/internal_modules/module-map
 */
import { notAvailableError } from "../notImplementedFail.js";
/** Job slot Node uses when an import names no `type` attribute. */
const IMPLICIT_TYPE_ATTRIBUTE = 'javascript';
/**
 * Node's `validateString` refusal.
 * @param value - Argument to check.
 * @param name - Argument name in the message.
 */
function validateString(value, name) {
    if (typeof value === 'string')
        return;
    throw Object.assign(new TypeError(`The "${name}" argument must be of type string. Received ${typeof value}`), {
        code: 'ERR_INVALID_ARG_TYPE',
    });
}
/** Resolutions keyed by parent URL, then by serialized module request. */
export class ResolveCache extends Map {
    /**
     * Serialize a module request: the specifier and its import attributes sorted by key.
     * @param specifier - Requested module.
     * @param importAttributes - Import attributes of the request.
     * @returns The cache key Node uses for this request.
     */
    serializeKey(specifier, importAttributes) {
        const keys = Object.keys(importAttributes);
        if (keys.length === 0)
            return `${specifier}::`;
        return `${specifier}::${keys.sort().map(key => JSON.stringify(key) + JSON.stringify(importAttributes[key])).join(',')}`;
    }
    #imports(parentURL) {
        const existing = super.get(parentURL);
        if (existing !== undefined)
            return existing;
        const created = Object.create(null);
        super.set(parentURL, created);
        return created;
    }
    /**
     * Read one cached resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @returns The cached resolution, if any.
     */
    get(serializedKey, parentURL) {
        return this.#imports(parentURL)[serializedKey];
    }
    /**
     * Store one resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @param result - Resolution to cache.
     * @returns This cache.
     */
    set(serializedKey, parentURL, result) {
        this.#imports(parentURL)[serializedKey] = result;
        return this;
    }
    /**
     * Check for one cached resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @returns Whether the request has a cached resolution.
     */
    has(serializedKey, parentURL) {
        return serializedKey in this.#imports(parentURL);
    }
}
/** Module jobs keyed by URL, then by import `type` attribute; always empty in the Worker. */
export class LoadCache extends Map {
    /**
     * Read the job for one URL and type.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @returns The cached job, if any.
     */
    get(url, type = IMPLICIT_TYPE_ATTRIBUTE) {
        validateString(url, 'url');
        validateString(type, 'type');
        return super.get(url)?.[type];
    }
    /**
     * Refuse to store a job: the Worker creates no ES module jobs.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @param _job - Job Node would store.
     * @returns Never; always throws.
     */
    set(url, type = IMPLICIT_TYPE_ATTRIBUTE, _job) {
        validateString(url, 'url');
        validateString(type, 'type');
        throw notAvailableError('internal/modules/esm/module_map', 'LoadCache.set');
    }
    /**
     * Check for the job of one URL and type.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @returns Whether a job is cached.
     */
    has(url, type = IMPLICIT_TYPE_ATTRIBUTE) {
        validateString(url, 'url');
        validateString(type, 'type');
        return super.get(url)?.[type] !== undefined;
    }
}
//# sourceMappingURL=module-map.js.map
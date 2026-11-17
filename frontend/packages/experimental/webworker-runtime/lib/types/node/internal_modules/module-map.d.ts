/** Resolutions keyed by parent URL, then by serialized module request. */
export declare class ResolveCache extends Map<unknown, unknown> {
    #private;
    /**
     * Serialize a module request: the specifier and its import attributes sorted by key.
     * @param specifier - Requested module.
     * @param importAttributes - Import attributes of the request.
     * @returns The cache key Node uses for this request.
     */
    serializeKey(specifier: string, importAttributes: Record<string, string>): string;
    /**
     * Read one cached resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @returns The cached resolution, if any.
     */
    get(serializedKey: string, parentURL?: unknown): unknown;
    /**
     * Store one resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @param result - Resolution to cache.
     * @returns This cache.
     */
    set(serializedKey: string, parentURL?: unknown, result?: unknown): this;
    /**
     * Check for one cached resolution.
     * @param serializedKey - Key from {@link serializeKey}.
     * @param parentURL - URL of the importing module.
     * @returns Whether the request has a cached resolution.
     */
    has(serializedKey: string, parentURL?: unknown): boolean;
}
/** Module jobs keyed by URL, then by import `type` attribute; always empty in the Worker. */
export declare class LoadCache extends Map<string, unknown> {
    /**
     * Read the job for one URL and type.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @returns The cached job, if any.
     */
    get(url: string, type?: string): unknown;
    /**
     * Refuse to store a job: the Worker creates no ES module jobs.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @param _job - Job Node would store.
     * @returns Never; always throws.
     */
    set(url: string, type?: unknown, _job?: unknown): never;
    /**
     * Check for the job of one URL and type.
     * @param url - Module URL.
     * @param type - Import `type` attribute.
     * @returns Whether a job is cached.
     */
    has(url: string, type?: string): boolean;
}
//# sourceMappingURL=module-map.d.ts.map
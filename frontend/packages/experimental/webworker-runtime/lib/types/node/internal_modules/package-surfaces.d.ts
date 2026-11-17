/** `Module` with the static members HMR reads and replaces. */
export declare const Module: (() => never) & {
    /** Empty: the Worker loader keeps its own resolution cache. */
    _pathCache: Record<string, string>;
    _resolveFilename: (...args: never[]) => never;
    _load: (...args: never[]) => never;
};
/** `internal/modules/package_json_reader` with the member HMR replaces. */
export declare const packageJsonReader: {
    getNearestParentPackageJSON: (...args: never[]) => never;
};
/** `internalBinding('modules')` with the members HMR replaces. */
declare const modulesBinding: {
    readPackageJSON: (...args: never[]) => never;
    getPackageScopeConfig: (...args: never[]) => never;
    getPackageType: (...args: never[]) => never;
};
/** `internal/bootstrap/realm`: only the `modules` binding is provided. */
export declare const realm: {
    /**
     * Answer one internal binding.
     * @param name - Binding name.
     * @returns The `modules` binding.
     * @throws Error with `ERR_FEATURE_UNAVAILABLE_ON_PLATFORM` for any other binding.
     */
    internalBinding(name: string): typeof modulesBinding;
};
export {};
//# sourceMappingURL=package-surfaces.d.ts.map
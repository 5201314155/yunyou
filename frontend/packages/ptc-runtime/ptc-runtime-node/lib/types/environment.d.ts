/** Native executable search, Windows system paths, and sandbox temporary paths retained in the OS environment. */
export declare const STARTUP_ENVIRONMENT_NAMES: ReadonlySet<string>;
/**
 * Drop warnings that the runtime itself triggers in the child, which would otherwise appear in program output; other warnings still print.
 * @param state - Child process whose `emitWarning` is wrapped; structural test doubles without it stay unchanged.
 */
export declare function filterRuntimeWarnings(state: {
    emitWarning?: typeof process.emitWarning;
}): void;
/**
 * Run a synchronous host call without printing the runtime's own experimental-feature warnings.
 * @param call - Synchronous work, such as type stripping, that may emit such a warning on first use.
 * @returns The call's result; `process.emitWarning` is restored before returning or rethrowing.
 */
export declare function quietRuntimeCall<T>(call: () => T): T;
/**
 * Clear inherited credentials while preserving native child-process startup paths.
 * With the native child process, preloaded node:process exports observe the replacement.
 * Isolated state objects only replace their own environment; they do not replace the native process.
 * @param state - Native child process in production, or isolated state in in-process tests.
 */
export declare function clearProgramEnvironment(state: {
    env: NodeJS.ProcessEnv;
}): void;
//# sourceMappingURL=environment.d.ts.map
/** Execution mode and resource limits for the Node PTC provider. */
import z from '@deepseek-ai/schemastery';
import type { LaunchConfig } from './launch.ts';
/** Deployment-varying runtime bounds and launch choices. */
export interface Config {
    /** Worker invocation in the subprocess world; omitted selects the local carrier. */
    launch?: LaunchConfig;
    /** per-call uses fresh async function bodies; vm-script uses VM contexts; persistent-in-memory retains a REPL per owner. */
    executionMode?: 'per-call' | 'persistent-in-memory' | 'vm-script';
    /** Retain a Script process and context per stateOwner; only supported with executionMode: vm-script. */
    scriptPersistence?: boolean;
    /**
     * Run persistent-in-memory programs as scoped REPL statements whose later calls receive copies of earlier top-level bindings;
     * omitted, it is `SCOPED_REPL_BY_DEFAULT` from configuration.ts, currently true, with persistent-in-memory and false otherwise.
     */
    replScoped?: boolean;
    /** Default elapsed deadline, including nested tool and approval waits. */
    timeoutMs?: number;
    /** Maximum numeric elapsed budget accepted by resolve. */
    maxTimeoutMs?: number;
    /** Combined serialized logs, completion and diagnostic byte cap. */
    maxOutputBytes?: number;
    /** V8 old-generation heap limit in MiB; native allocations are excluded. */
    maxOldGenerationSizeMb?: number;
    /** Maximum control message, outstanding binding-argument and queued control-output bytes. */
    maxMessageBytes?: number;
    /** Maximum simultaneous host binding calls accepted from a program. */
    maxPendingCalls?: number;
    /** Time to wait for cooperative stop acknowledgment before terminating the process. */
    stopGraceMs?: number;
    /** Managed process termination and output-drain grace in milliseconds. */
    graceMs?: number;
}
/** Configuration after Loader defaults and executable selection. */
export type ResolvedConfig = Required<Config>;
/** Loader defaults for the Node PTC provider. */
export declare const NodeConfig: z<Config>;
/**
 * Select the scoped REPL setting for the configured execution mode.
 * @param config - Loader-defaulted fields whose `replScoped` may be omitted.
 * @returns The explicit setting; when omitted, the setting that `resolvePersistentExecution` writes for function bodies with
 * `persistent-in-memory`, and false in other modes.
 */
export declare function resolveReplScoped(config: Pick<Config, 'executionMode' | 'replScoped'>): boolean;
/**
 * Validate resolved Node limits and paths.
 * @param config - Loader-defaulted fields with the selected executable.
 */
export declare function validateConfig(config: ResolvedConfig): void;
//# sourceMappingURL=config.d.ts.map
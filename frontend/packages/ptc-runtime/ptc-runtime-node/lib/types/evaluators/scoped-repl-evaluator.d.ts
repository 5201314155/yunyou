import type { ProgramEvaluator, ProgramEvaluatorOptions } from './types.ts';
/** Environment choices for one scoped REPL evaluator; omitted fields use the documented defaults. */
export interface ScopedReplEvaluatorOptions extends ProgramEvaluatorOptions {
    /** Force strict mode for every program; when off, a program's own leading "use strict" decides. Defaults to true. */
    strict?: boolean;
    /** Allow eval, new Function and WebAssembly compilation inside the program context; defaults to false. */
    codeGeneration?: boolean;
    /** Use the last top-level expression statement as the completion value; defaults to true. */
    finalExpression?: boolean;
}
/**
 * Create an evaluator whose programs see copies of earlier top-level bindings and may declare any of them again.
 * Each program is a separate Script in one retained context; the scope chain lives in {@link ScopedReplScopes}.
 * Programs load modules only with dynamic `import()`; there is no `require`, `module`, or built-in module global.
 * Relative specifiers resolve from the working directory at creation, also after later calls change directory.
 * A final Promise is returned without being awaited. Asynchronous program failures go to `reportError` through a domain,
 * as in Node's REPL, instead of terminating the process.
 * @param options - Runner diagnostics, optional import handling, and environment choices.
 * @returns Context globals, sequential evaluation, and resource cleanup.
 */
export declare function createScopedReplEvaluator(options: ScopedReplEvaluatorOptions): ProgramEvaluator;
//# sourceMappingURL=scoped-repl-evaluator.d.ts.map
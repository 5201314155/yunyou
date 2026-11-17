/** Read program source and language hints from recorded tool arguments and schemas. */
import type { TrajectoryCellProps } from './trajectory-record.ts';
/**
 * Identify tools supported by the trajectory code inspector.
 * @param name - Recorded tool name, absent when the call is outside the loaded window.
 * @returns Whether this tool executes a program shown by the code inspector.
 */
export declare function isProgramTool(name: string | undefined): boolean;
/** Validated source and original arguments for one recorded program call. */
export interface CodeProgram {
    rawInput: string;
    source: string;
    description: string;
    arguments: Record<string, unknown>;
    language: 'typescript' | 'python' | undefined;
}
/**
 * Resolve a program without guessing its language from source or current runtime settings.
 * Unknown explicit languages or conflicting recorded hints leave the source unhighlighted.
 * @param cell - Recorded tool arguments and the schema visible at call time.
 * @returns The program, or undefined for another tool or unsupported arguments.
 */
export declare function codeProgram(cell: TrajectoryCellProps): CodeProgram | undefined;
//# sourceMappingURL=code-program.d.ts.map
import { type Context, type ScriptOptions } from 'node:vm';
/** Declaration keyword recorded for a top-level binding; later programs copy `var` bindings as `var` and every other binding as `let`. */
export type ScopedReplBindingKind = 'var' | 'let' | 'const' | 'function' | 'class';
/** One top-level binding passed from earlier programs to later ones. */
export interface ScopedReplBinding {
    /** Binding identifier. */
    name: string;
    /** Keyword of the declaration that created the binding. */
    kind: ScopedReplBindingKind;
}
/** Source offsets of the last top-level expression statement. */
export interface ScopedReplFinalExpression {
    /** Offset where the statement begins, including a leading parenthesis. */
    statementStart: number;
    /** Offset where the expression begins. */
    expressionStart: number;
    /** Offset where the expression ends, before a trailing parenthesis or semicolon. */
    expressionEnd: number;
}
/** Own declarations and completion position found by one scan. */
export interface ScopedReplScan {
    /** Top-level bindings declared by the program, in source order. */
    declarations: ReadonlyMap<string, ScopedReplBindingKind>;
    /** Whether the program's leading directives include "use strict". */
    useStrict: boolean;
    /** Last top-level statement when it is an expression statement. */
    finalExpression?: ScopedReplFinalExpression;
}
/** Prefix of every generated name; programs that contain it anywhere are rejected. */
export declare const RESERVED_PREFIX = "__$binding_";
/** Generated parameter receiving the values copied from earlier programs. */
export declare const BINDING_VALUES = "__$binding_vals";
/** Generated parameter receiving one read function per binding, in binding order. */
export declare const BINDING_GETTERS = "__$binding_getters";
/**
 * Parse a program once and list its own top-level bindings, leading strict directive, and final expression.
 * Every program uses script rules with top-level await: strictness comes from the generated Script or the program's directive,
 * and V8 reports strict-mode violations when compiling. Script rules reject top-level `using` and static `import`.
 * Top-level declarations are read, and so are var declarations inside blocks and other statements, which hoist to the program's scope;
 * declarations inside nested functions and classes are not read.
 * @param code - Type-stripped JavaScript submitted for one evaluation.
 * @param options - Filename shown by syntax errors.
 * @returns Declarations in source order, whether a "use strict" directive leads, and the final expression offsets.
 * @throws {SyntaxError} Laid out like V8's for parse errors, top-level return, a hashbang, and any occurrence of {@link RESERVED_PREFIX}.
 */
export declare function scanProgram(code: string, options: {
    filename: string;
}): ScopedReplScan;
/**
 * Build the Script source that copies earlier bindings, exposes read functions, and runs the program.
 * The generated prefix occupies line 0, so compiling with `lineOffset: -1` reports program line N as line N.
 * A copy exists exactly for each carried binding. A name the program declares again starts from its earlier value through a `var` copy,
 * other `var` bindings are copied as `var`, and all others as `let`.
 * With `strict` off, a program whose directives include "use strict" gets that directive at the start of its function body.
 * @param code - Program text that produced `scan`.
 * @param scan - Result of {@link scanProgram} for `code`.
 * @param carried - Earlier bindings to copy, in the order of the copied values: names the program does not declare,
 * and names it declares again only with `var`.
 * @param options - Whether the whole Script is strict, and whether the final expression becomes the completion value.
 * @returns Source evaluating to a factory, and every binding in the order of the read functions it pushes.
 */
export declare function buildProgramSource(code: string, scan: ScopedReplScan, carried: readonly ScopedReplBinding[], options: {
    strict: boolean;
    finalExpression: boolean;
}): {
    source: string;
    bindings: ScopedReplBinding[];
};
/** A name declared again after an earlier program bound it. */
export interface ScopedReplRedeclaration {
    /** Redeclared identifier. */
    name: string;
    /** Keyword recorded for the earlier binding: its most recent declaration, kept by programs that only copied it. */
    previous: ScopedReplBindingKind;
    /** Keyword of the redeclaring program's declaration. */
    current: ScopedReplBindingKind;
}
/** Completion of one program: the boxed final expression, or undefined when there is none or it is disabled. */
export type ScopedReplCompletion = {
    value: unknown;
} | undefined;
/** A compiled program bound to its copied values; each call runs the program body once. */
export type ScopedReplProgram = () => Promise<ScopedReplCompletion>;
/** Choices fixed for every program of one scope chain. */
export interface ScopedReplScopesOptions {
    /** Force strict mode for every program; otherwise a program's own leading "use strict" directive decides. */
    strict: boolean;
    /** Receives the names a successfully compiled program declares again, before that program runs. */
    onRedeclaration?: (redeclarations: readonly ScopedReplRedeclaration[]) => void;
}
/** Inputs for compiling one program. */
export interface ScopedReplCompileOptions {
    /** Filename shown by syntax errors and stack frames of this program. */
    filename: string;
    /** Context shared by every program of the chain. */
    context: Context;
    /** Use the last top-level expression statement as the boxed completion. */
    finalExpression: boolean;
    /** Dynamic import handling for this program; omission leaves import() without a callback. */
    importModuleDynamically?: ScriptOptions['importModuleDynamically'];
}
/**
 * Compile programs that each run in their own scope and start with copies of the top-level bindings earlier programs left.
 * A program reads earlier bindings when it is compiled: values earlier code assigns afterwards stay with that code.
 * A var redeclaration starts from the earlier value. An earlier name whose let, const, or class redeclaration never finished initializing
 * keeps the value it had before; an uninitialized new name is dropped.
 */
export declare class ScopedReplScopes {
    private readonly options;
    private bindings;
    private replaced;
    private compiled;
    /**
     * Start an empty scope chain.
     * @param options - Strictness and the redeclaration observer shared by every program.
     */
    constructor(options: ScopedReplScopesOptions);
    /**
     * Scan and compile one program against the bindings current at this moment.
     * Bindings of the previously compiled program apply once its body has started; a syntax error leaves the chain unchanged.
     * @param code - Type-stripped JavaScript of the program.
     * @param options - Filename, shared context, completion choice, and import handling.
     * @returns The program, which the caller runs once.
     * @throws {SyntaxError} From scanning or V8 compilation, with its location and caret and without compiler frames.
     */
    compile(code: string, options: ScopedReplCompileOptions): ScopedReplProgram;
    private adoptCompiled;
    private currentValue;
}
//# sourceMappingURL=scoped-repl.d.ts.map
/**
 * Detection of workspace build outputs loaded into an analyzed face program,
 * and of workspace imports that resolve to nothing. Typert reads workspace
 * packages only from source when a caller rejects build outputs, so its
 * result cannot depend on whether, or when, package compilation ran: an
 * import that reaches `lib/` on a compiled tree resolves to nothing on a
 * clean one, and both are rejected.
 * @module @deepseek-ai/dsh-typert-generator/build-output-guard
 */
import ts from 'typescript';
/** One program file that a workspace package's `lib/` holds, with the imports that load it. */
export interface BuildOutputLoad {
    /** Absolute path of the build output. */
    readonly file: string;
    /** Program files outside build outputs whose import specifier resolves to {@link file}. */
    readonly importers: readonly BuildOutputImporter[];
    /** Absolute path of the workspace source compiled to {@link file}, when one exists. */
    readonly source: string | undefined;
}
/** One import of a build output from a program file outside build outputs. */
export interface BuildOutputImporter {
    /** Absolute path of the importing file. */
    readonly file: string;
    /** Import specifier as written. */
    readonly specifier: string;
}
/**
 * List the program files that a workspace package holds under its `lib/`
 * directory. A package is the nearest directory with a `package.json` at or
 * below `root`; files under `node_modules` are external. Importers are found
 * by resolving each program file's import specifiers again, so this pass
 * costs extra only when an output is present.
 * @param program - the analyzed face program.
 * @param root - workspace root.
 * @param host - module resolution host of the program.
 * @returns loads in file-path order; empty when every workspace file comes from source.
 */
export declare function findBuildOutputLoads(program: ts.Program, root: string, host: ts.ModuleResolutionHost): BuildOutputLoad[];
/** One import of a workspace package from a program source file that module resolution cannot resolve. */
export interface UnresolvedWorkspaceImport {
    /** Absolute path of the importing file. */
    readonly file: string;
    /** Import specifier as written. */
    readonly specifier: string;
    /** Absolute path of the workspace source the specifier's package export compiles from, when one exists. */
    readonly source: string | undefined;
}
/**
 * List the imports in workspace program files that name a workspace package
 * and resolve to nothing. A workspace package is one whose `node_modules` link,
 * found from the importing file's directories, leads inside `root` outside any
 * `node_modules`; without that link or a `paths` alias the package cannot be
 * identified, and type checking reports the import. The source of a subpath is
 * derived from the package's `exports` `types` condition under `lib/types/`.
 * @param program - the analyzed face program.
 * @param root - workspace root.
 * @param host - module resolution host of the program.
 * @returns unresolved imports in file and specifier order.
 */
export declare function findUnresolvedWorkspaceImports(program: ts.Program, root: string, host: ts.ModuleResolutionHost): UnresolvedWorkspaceImport[];
//# sourceMappingURL=build-output-guard.d.ts.map
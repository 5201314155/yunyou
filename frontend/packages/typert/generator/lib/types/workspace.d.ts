/**
 * Workspace-level discovery and model-driven Typert generation.
 * @module @deepseek-ai/dsh-typert-generator/workspace
 */
import type { DiscoveredTypertPackage } from './analyzer.ts';
import type { ModelEmitResult } from './emitter.ts';
import type { TypertFace } from './model.ts';
/** One emitted artifact paired with its source package root. */
export interface WorkspaceEmitResult extends ModelEmitResult {
    readonly packageRoot: string;
}
/** Behavior switches for one {@link WorkspaceTypertGenerator}. */
export interface WorkspaceTypertGeneratorOptions {
    /**
     * Run the per-package syntactic/semantic diagnostic pass before analysis
     * (default true). Pass false only when the same orchestration already
     * verified the workspace with tsc; the Typert-specific analysis checks
     * (annotation coverage, private cross-package references, unretainable
     * merges) run regardless.
     */
    readonly checkDiagnostics?: boolean;
    /**
     * Fail when an analyzed face program loads a file from a workspace
     * package's `lib/` directory or cannot resolve an import of a workspace
     * package (default false); see
     * `WorkspaceAnalyzerOptions.rejectBuildOutputs`.
     */
    readonly rejectBuildOutputs?: boolean;
}
/** Discover, analyze, and emit package reflection from independent faces. */
export declare class WorkspaceTypertGenerator {
    private readonly root;
    private readonly options;
    /** Parsed-config and program-host state shared by every analyzer this generator creates. */
    private readonly caches;
    /**
     * Bind generation to one workspace root.
     * @param root - directory containing face aggregate tsconfigs.
     * @param options - behavior switches applied to every pass of this generator.
     */
    constructor(root: string, options?: WorkspaceTypertGeneratorOptions);
    /**
     * Find public package faces that contribute Cordis services/events or
     * explicitly tagged Typert roots.
     * @param faces - optional independent program faces to inspect.
     * @param includeManifest - optional `package.json` predicate; a rejected package's sources and project config are not read.
     * @returns discovered packages in stable package-name order.
     */
    discover(faces?: readonly TypertFace[], includeManifest?: (manifest: Readonly<Record<string, unknown>>) => boolean): DiscoveredTypertPackage[];
    /**
     * Generate all discovered contributors, or an explicit package subset.
     * @param packages - optional exact package names to emit; every pass includes the same face-wide declaration merges.
     * @param faces - optional independent program faces to analyze.
     * @returns one artifact per package face.
     */
    generate(packages?: readonly string[], faces?: readonly TypertFace[]): WorkspaceEmitResult[];
    /**
     * Generate every explicit contributor: a package whose manifest declares a
     * Typert artifact export and whose public graph has a Typert surface.
     * @param faces - optional independent program faces to analyze.
     * @returns one artifact per contributor face.
     */
    generateContributors(faces?: readonly TypertFace[]): WorkspaceEmitResult[];
    private validateExport;
}
/**
 * Find the workspace root that Typert generation runs against: the nearest
 * directory at or above `start` that holds both `tsconfig.host.json` and
 * `packages/`. The analyzer registers projects under `<root>/packages`, so a
 * package's own `tsconfig.host.json` face leaf does not qualify.
 * @param start - directory to search from, such as a bundle output or a working directory.
 * @returns the absolute workspace root.
 * @throws If no ancestor of `start` qualifies.
 */
export declare function findWorkspaceRoot(start: string): string;
/**
 * Whether a `package.json#exports` field declares a Typert artifact subpath
 * (`./typert`, `./client/typert`, or `./remote`).
 * @param exportsField - the manifest's `exports` value.
 * @returns true when the package opts in to Typert publication.
 */
export declare function hasTypertExport(exportsField: unknown): boolean;
//# sourceMappingURL=workspace.d.ts.map
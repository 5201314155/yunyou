/**
 * TypeScript project analyzer for the compiler-independent Typert model.
 * Programs, symbols, and syntax nodes remain extraction-only implementation
 * details; callers receive only the model declared in {@link ./model.ts}.
 * @module @deepseek-ai/dsh-typert-generator/analyzer
 */
import ts from 'typescript';
import type { SourceDeclarationModel, TypertFace, WorkspaceModel } from './model.ts';
/** Analysis failure with a source-oriented diagnostic. */
export declare class TypertAnalysisError extends Error {
    name: string;
}
/** Missing-annotation handling at public business boundaries. */
export type AnalysisMode = 'check' | 'write';
/** Workspace analysis configuration. */
export interface WorkspaceAnalyzerOptions {
    /** Workspace root containing the face tsconfigs. */
    readonly root: string;
    /** Host aggregate path, relative to {@link root}; absent files are skipped. */
    readonly hostConfig?: string;
    /** Client aggregate path, relative to {@link root}; absent files are skipped. */
    readonly clientConfig?: string;
    /** Optional package-name subset for an incremental generation pass. */
    readonly packages?: readonly string[];
    /** Independently compiled faces to materialize; both are analyzed by default. */
    readonly faces?: readonly TypertFace[];
    /** Whether to repeat TypeScript project diagnostics before model extraction. */
    readonly checkDiagnostics?: boolean;
    /** Whether missing annotations fail or are written before a clean re-analysis. */
    readonly mode?: AnalysisMode;
    /** Shared workspace memo; supply one instance to reuse parses across analyzers. */
    readonly caches?: WorkspaceCaches;
    /**
     * Fail analysis when a face program loads a file from a workspace
     * package's `lib/` directory, or cannot resolve an import of a workspace
     * package (default false). Enable it for faces that import no generated
     * output, so that the result depends only on workspace source and never on
     * whether package compilation ran; the failure names each importing
     * specifier and the source a `paths` alias should map it to.
     */
    readonly rejectBuildOutputs?: boolean;
}
/** One package face whose public export graph contains Typert business declarations. */
export interface DiscoveredTypertPackage {
    readonly package: string;
    readonly root: string;
    readonly faces: readonly TypertFace[];
}
/** One parsed tsconfig, memoizable per workspace snapshot. */
export interface ParsedConfig {
    /** Absolute config path. */
    readonly path: string;
    /** The TypeScript parse result. */
    readonly parsed: ts.ParsedCommandLine;
}
/** The settings Typert reads from one face aggregate tsconfig. */
export interface AggregateConfig {
    /** Absolute config path. */
    readonly path: string;
    /** The aggregate's effective compiler options. */
    readonly options: ts.CompilerOptions;
    /** The package projects the aggregate references. */
    readonly projectReferences: readonly ts.ProjectReference[];
}
/** One package face registration discovered from an aggregate tsconfig. */
export interface PackageRegistration {
    /** The face whose aggregate references this package project. */
    readonly face: TypertFace;
    /** The package manifest name. */
    readonly name: string;
    /** Real package root directory. */
    readonly root: string;
    /**
     * The package's own tsconfig, parsed on first read. Typert parses only the
     * tsconfigs of packages it analyzes; validating every referenced tsconfig
     * belongs to the preceding `tsc -b`.
     */
    readonly config: ParsedConfig;
    /** The parsed package.json content. */
    readonly manifest: Record<string, unknown>;
    /** Export subpaths owned by this face for dual-face packages. */
    readonly exportSubpaths?: readonly string[];
}
/**
 * Shared memo over one immutable workspace snapshot. Passing one instance to
 * several analyzers (the batched and write-mode children reuse their parent's
 * automatically) reuses parsed tsconfigs, the registration inventory, and
 * per-face compiler hosts whose parsed and bound source files and module
 * resolutions carry across programs. Callers that mutate workspace files
 * between analyses must start from a fresh instance; write-mode source edits
 * invalidate themselves through {@link invalidate}.
 */
export declare class WorkspaceCaches {
    /** Parsed tsconfig files by absolute config path. */
    readonly configs: Map<string, ParsedConfig>;
    /** Registration inventories keyed by root and aggregate config paths. */
    readonly registrations: Map<string, PackageRegistration[]>;
    private readonly aggregates;
    private readonly hosts;
    /**
     * Parse one tsconfig once per workspace snapshot.
     * @param path - absolute config path.
     * @returns the memoized parse result.
     */
    config(path: string): ParsedConfig;
    /**
     * Read one face aggregate tsconfig once per workspace snapshot. An aggregate
     * that declares `references` is parsed without expanding its `include`
     * patterns, which affect neither its options nor its references.
     * @param path - absolute aggregate config path.
     * @returns the memoized compiler options and project references.
     */
    aggregate(path: string): AggregateConfig;
    /**
     * Return the shared compiler host for one face. Every program of one face
     * is built from the same aggregate compiler options (the first call wins),
     * so parsed source files, binder state, and module resolutions are safe to
     * reuse across the face's batched programs.
     * @param face - the face whose programs share this host.
     * @param options - the face's effective compiler options.
     * @returns a compiler host with source-file and module-resolution caches.
     */
    programHost(face: TypertFace, options: ts.CompilerOptions): ts.CompilerHost;
    /**
     * Drop cached parses of one edited source file so the next analysis reads
     * the written content.
     * @param file - path of the edited file.
     */
    invalidate(file: string): void;
}
/** Analyze host and client as independent TypeScript programs. */
export declare class WorkspaceAnalyzer {
    private readonly options;
    private queuedEdit;
    private readonly crossFaceLinks;
    private readonly checkedProjects;
    private registrations;
    private readonly caches;
    constructor(options: WorkspaceAnalyzerOptions);
    /**
     * Build the workspace model. Write mode applies inferred annotations and then
     * returns a fresh check-mode analysis of the edited projects.
     * @returns the independent face models and their explicit cross-face links.
     */
    analyze(): WorkspaceModel;
    /**
     * Analyze an explicit package selection through bounded compiler programs.
     * The resulting model is identical in shape to {@link analyze}; stable graph
     * ids let repeated dependency declarations merge without flattening types.
     * @param batchSize - maximum selected packages in one face program.
     * @returns one merged workspace model.
     */
    analyzeInBatches(batchSize?: number): WorkspaceModel;
    /**
     * Discover package faces from public-export-reachable Cordis augmentations
     * and explicit `@typert` roots without constructing a type-checker program.
     * @param includeManifest - optional `package.json` predicate; a rejected package's sources and project config are not read.
     * @returns contributors grouped by package with deterministic face order.
     */
    discoverPackages(includeManifest?: (manifest: Readonly<Record<string, unknown>>) => boolean): DiscoveredTypertPackage[];
    /**
     * Index top-level exported type declarations without promoting them to graph
     * roots. Consumers use this lexical index for ambiguity checks while all
     * semantic traversal continues through {@link TypeGraph}.
     * @returns declarations from the selected faces and package projects.
     */
    indexSourceDeclarations(): SourceDeclarationModel[];
    private loadRegistrations;
    private entrySourcePaths;
    private registrationHasSurface;
    private checkProject;
    private queueEdit;
    private applyEdit;
}
//# sourceMappingURL=analyzer.d.ts.map
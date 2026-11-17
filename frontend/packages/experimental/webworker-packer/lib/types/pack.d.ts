import { type PatchOptions } from '@deepseek-ai/cordis-plugin-include';
import { type ImageFiles, type TransformOutcome } from './transform-image.ts';
export { DEFAULT_ROOT } from '@deepseek-ai/dsh-experimental-webworker-runtime';
/** Image path of the manifest; the layout contract's name, re-exported for callers. */
export declare const MANIFEST_PATH: string;
/** Image path of the home patch: `$DSH_HOME/cordis.patch.yml` under the Worker home. */
export declare const HOME_PATCH_PATH = "home/cordis.patch.yml";
/** One directory tree to copy into the image at a caller-selected mount. */
export interface ImageTree {
    /** Image path to mount it at, relative to the virtual root. */
    readonly mount: string;
    /** Absolute source directory. */
    readonly directory: string;
}
/** One configuration tree whose plugin rows may extend the package roster. */
export interface ConfigTree extends ImageTree {
    /**
     * Whether plugin names inside its `.yml` files join the materialization closure.
     * An agent preset mounts plugins the base composition never lists, and creating a
     * session fails if any of them is missing from the image.
     */
    readonly scanRoster?: boolean;
}
/** The installation whose manifest alone anchors bundle resolution inside the image. */
export interface ImageInstallation {
    /** Package name the manifest is mounted under, `node_modules/<name>/package.json`. */
    readonly name: string;
    /** Absolute path of the manifest to copy. */
    readonly manifest: string;
}
/** Everything the packer needs that it cannot know by itself. */
export interface PackOptions {
    /** Shipped profile template the worker host boots, recorded in the manifest. */
    readonly profile: string;
    /**
     * Package names whose dependency closures the image materializes: the bundles the
     * image offers and every package their patch rows name.
     */
    readonly roster: readonly string[];
    /** Installation manifest copied alone, without its package or dependency closure. */
    readonly installation?: ImageInstallation;
    /** Virtual root the image mounts under; defaults to {@link DEFAULT_ROOT}. */
    readonly root?: string;
    /** Package name to absolute directory, for workspace and vendored packages. */
    readonly workspaces: ReadonlyMap<string, string>;
    /** Directory Node-style dependency resolution walks up from for the roster. */
    readonly resolveFrom: string;
    /** Config trees to copy in beside the composition. */
    readonly configTrees?: readonly ConfigTree[];
    /** Empty directories to create; defaults to `home/`, `workspace/`, `tmp/`. */
    readonly emptyDirectories?: readonly string[];
    /** Patch rows written to {@link HOME_PATCH_PATH}, which the image's app-boot reads as the home layer. */
    readonly homePatch?: readonly PatchOptions[];
    /**
     * Extra sweep roots: image specifiers requested by code outside the image.
     * Defaults to the worker assembly's own entries.
     */
    readonly entries?: readonly string[];
}
/** One lazy chunk archive to write beside the base image. */
export interface PackedLazyArchive {
    /** Path relative to the base image, `lazy/<sha256 of image>.tar.gz`; the digest names the archive, and the Worker does not verify it. */
    readonly name: string;
    /** The gzip-compressed tar archive. */
    readonly image: Uint8Array;
    /** Image paths the archive carries. */
    readonly files: readonly string[];
}
/** What one pack produced, for the caller to report or assert on. */
export interface PackResult {
    /** The gzip-compressed tar archive to write; the runtime inflates it at mount. */
    readonly image: Uint8Array;
    /** Every base-image entry, before zipping; the manifest is already among them. */
    readonly files: ImageFiles;
    /** Page chunks moved out of the base image, one archive each, in path order. */
    readonly lazyArchives: readonly PackedLazyArchive[];
    /** Package name to how many files it contributed, in materialization order. */
    readonly packages: ReadonlyMap<string, number>;
    /** How many of them came from the workspace rather than from `node_modules`. */
    readonly workspacePackages: number;
    /** Roster package names the closure started from. */
    readonly roster: readonly string[];
    /** Dependencies that did not resolve; a non-empty list means an incomplete image. */
    readonly missing: readonly string[];
    /** Executable scripts dropped from the image. */
    readonly executables: readonly string[];
    /** Page bundles left out of the transform; like every JavaScript entry they carry the trailing debugger name. */
    readonly pageBundles: readonly string[];
    /** JavaScript entries the image carries. */
    readonly javascriptEntries: number;
    /** JavaScript candidates no root reaches, dropped from the image. */
    readonly droppedJavascriptEntries: number;
    /** Third-party requests that resolve nowhere; loud at require time if hit. */
    readonly unresolvedExternalRequests: readonly string[];
    /** What the pack-time transform did. */
    readonly transform: TransformOutcome;
    /** Wrapper contract recorded in the manifest; every packed body meets it. */
    readonly contract: string;
}
/** One deterministic data-overlay archive and its uncompressed entries. */
export interface PackOverlayResult {
    /** Gzip-compressed ustar bytes consumed by the Worker host. */
    readonly image: Uint8Array;
    /** Every path in the overlay before compression. */
    readonly files: ImageFiles;
}
/**
 * Collect package names from entry rows and patch lists, recursively through
 * `insert` lists and nested `config` row lists (groups). Builtin rows
 * (`cordis:group`), preset metadata documents, and file-relative entries carry
 * names that are not package specifiers; only scoped names and bare names
 * with a subpath count.
 * @param rows - Parsed YAML value; anything but a row array is ignored.
 * @param names - Package names collected so far.
 */
export declare function collectModuleNames(rows: unknown, names: Set<string>): void;
/**
 * Pack one VFS image.
 *
 * The manifest's claim is all-or-nothing: it names the one contract every packed body
 * was emitted against. A module the transform cannot express therefore fails the pack
 * rather than downgrading the image, because a mostly-transformed image boots into
 * errors far from their cause. Page chunks requested through `require.async` leave
 * the base image for deterministic archives named by their content digest, which the manifest registers.
 * @param options - Roster, installation, package index, and paths.
 * @returns The compressed image, its lazy archives, and what went into them.
 * @throws When a config tree or workspace directory named in the options is missing,
 * because a silently thinner image fails much later and much less clearly; when the
 * installation is also a materialized package; or when a page asset requests a chunk
 * its package does not publish.
 */
export declare function packVfsImage(options: PackOptions): PackResult;
/**
 * Pack opaque data trees into one ordered VFS overlay.
 *
 * Overlay mounts are restricted to the runtime-owned data directories, so an
 * overlay cannot replace configuration, the lowering manifest, or modules.
 * Files bypass package excludes and module reachability processing; later
 * trees replace earlier files at the same path.
 * @param trees - Absolute source directories and their data-directory mounts.
 * @returns Deterministic compressed archive plus its uncompressed entries.
 */
export declare function packVfsOverlay(trees: readonly ImageTree[]): PackOverlayResult;
//# sourceMappingURL=pack.d.ts.map
import type { PatchOptions } from '@deepseek-ai/cordis-plugin-include';
import { type ConfigTree, type ImageInstallation, type ImageTree, type PackResult } from './pack.ts';
export { packPreviewFixture } from './preview.ts';
/** One built-in Preview source and the trees packed into its overlay. */
export interface PreviewFixture {
    /** URL/query-safe identifier. */
    readonly id: string;
    /** User-facing chooser label. */
    readonly label: string;
    /** User-facing chooser detail. */
    readonly description: string;
    /** Opaque trees packed into this fixture's overlay archive. */
    readonly trees: readonly ImageTree[];
}
/**
 * Index every workspace and vendored package by name.
 * @param repoRoot - Absolute repository root.
 * @returns Package name to absolute directory.
 */
export declare function indexWorkspacePackages(repoRoot: string): Map<string, string>;
/** The bundles one image offers and the packages it must materialize for them. */
export interface ProfileRoster {
    /** The profile template's bundles, then the installation's optional bundles. */
    readonly bundles: readonly string[];
    /** Those bundle packages plus every package their patch rows name, deduplicated. */
    readonly roster: readonly string[];
}
/**
 * Read what one shipped profile offers from the installation the launcher boots:
 * the template's bundles and every optional bundle, each resolved from the CLI
 * manifest as the launcher resolves them, and the packages their patch files name.
 * Machine state never contributes, so the same tree always packs the same roster.
 * @param repoRoot - Absolute repository root.
 * @param profile - Shipped profile template name.
 * @returns Bundle names and the roster to materialize.
 * @throws When the profile has no shipped template, or a bundle does not resolve or declares no patch.
 */
export declare function profileRoster(repoRoot: string, profile: string): ProfileRoster;
/**
 * The home patch one shipped profile's image carries: every row `PLATFORM_ENTRY_CONFIG` names, with the config the
 * template's bundle layers compose for it and the Worker's required fields over that config. A patch replaces a row's
 * whole config, so the composed fields stay; the image carries the same bundle files read here.
 * @param repoRoot - Absolute repository root.
 * @param profile - Shipped profile template name.
 * @returns Home patch rows; a row the composition lacks contributes none.
 * @throws When the profile has no shipped template, or a bundle does not resolve or declares no patch.
 */
export declare function platformHomePatch(repoRoot: string, profile: string): PatchOptions[];
/**
 * The installation manifest the image anchors bundle resolution at.
 * @param repoRoot - Absolute repository root.
 * @returns The CLI package name and its manifest path.
 */
export declare function installationManifest(repoRoot: string): ImageInstallation;
/**
 * Pack this repository's base image for one shipped profile.
 * @param repoRoot - Absolute repository root.
 * @param profile - Shipped profile template name.
 * @param root - Virtual root; the runtime default when omitted.
 * @returns The packed image and its lazy archives.
 */
export declare function packRepositoryImage(repoRoot: string, profile: string, root?: string): PackResult;
/**
 * Config trees the CLI package declares for deployment images
 * (`dsh.configTrees` in its package.json): `path` is relative to the CLI
 * package root, `mount` is the image path, `scanRoster` feeds the tree's yml
 * plugin rows into the pack roster. The CLI owns its config layout; this
 * reader follows the declaration instead of naming directories. A malformed
 * declaration refuses the pack.
 * @param repoRoot - Absolute repository root.
 * @returns Trees with absolute source directories.
 */
export declare function configTrees(repoRoot: string): ConfigTree[];
/**
 * Built-in filesystem fixtures offered by the repository preview.
 * Session and Workspace semantics remain opaque here; the owning runtime tests
 * validate those files through their production readers.
 * @param repoRoot - Absolute repository root.
 * @returns Named chooser entries and their overlay trees.
 */
export declare function previewFixtures(repoRoot: string): PreviewFixture[];
/**
 * Render one pack as the lines a build log should carry.
 *
 * Refusals and unresolved dependencies are the two states a reader must not miss, so
 * they are spelled out rather than counted.
 * @param result - What the pack produced.
 * @param repoRoot - Absolute repository root, for relative paths.
 * @param outputFile - Where the image was written.
 * @returns Lines to print.
 */
export declare function describePack(result: PackResult, repoRoot: string, outputFile: string): string[];
//# sourceMappingURL=repository.d.ts.map
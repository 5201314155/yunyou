import type { MemoryVfs } from './storage/memory.ts';
/**
 * Archive every file a package of the image publishes through `"./assets/*": "./lib/assets/*"`.
 * @param vfs - Mounted image filesystem.
 * @param root - Image root holding `node_modules`.
 * @returns An uncompressed tar whose entries are named `plugins/<package>/assets/<path>` with the path unencoded,
 * so every name fits the ustar limits the base image already met.
 */
export declare function packPackageAssets(vfs: MemoryVfs, root: string): Uint8Array;
/**
 * Map each archived package asset to a Blob URL kept for the page's lifetime.
 * @param archive - Archive from {@link packPackageAssets}.
 * @returns The synchronous resolver behind `require.asset`: the Blob URL of a
 * reference the image holds, otherwise the reference itself, which then fails as an unserved image.
 */
export declare function packageAssetResolver(archive: Uint8Array): (reference: string) => string;
//# sourceMappingURL=package-assets.d.ts.map
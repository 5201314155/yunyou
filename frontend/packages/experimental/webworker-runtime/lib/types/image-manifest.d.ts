/** One file a lazy archive supplies. */
export interface ImageLazyFile {
    /** Path relative to the image root, as the archive names it. */
    readonly path: string;
    /** Byte length of the archived file. */
    readonly size: number;
}
/** One archive beside the base image whose files arrive on first asynchronous read. */
export interface ImageLazyArchive {
    /** Archive path relative to the base image URL. */
    readonly archive: string;
    /** Every file the archive carries. */
    readonly files: readonly ImageLazyFile[];
}
/** Fields recorded in the base image's manifest. */
export interface ImageManifest {
    /** Virtual root the image was packed for. */
    readonly root: string;
    /** Shipped profile template the worker host initializes and boots. */
    readonly profile: string;
    /** Package whose manifest alone anchors bundle resolution; absent in images that boot no tree. */
    readonly installation?: string;
    /** Wrapper contract every packed body was lowered against. */
    readonly lowered: string;
    /** JavaScript entries the image carries, lazy archives excluded. */
    readonly javascriptEntries: number;
    /** Entries the pack-time transform visited. */
    readonly visitedEntries: number;
    /** Visited entries the transform rewrote. */
    readonly rewrittenEntries: number;
    /** Page chunks requested only through `require.async`, one archive each. */
    readonly lazy: readonly ImageLazyArchive[];
}
/**
 * Validate a fetched base-image manifest.
 * @param value - Parsed manifest JSON.
 * @param path - Manifest path inside the image, named in refusals.
 * @returns The detached manifest.
 * @throws When the manifest is malformed, or its bodies were lowered against another wrapper contract.
 */
export declare function parseImageManifest(value: unknown, path: string): ImageManifest;
//# sourceMappingURL=image-manifest.d.ts.map
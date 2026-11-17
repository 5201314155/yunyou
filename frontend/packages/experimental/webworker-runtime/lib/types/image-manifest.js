/**
 * The base image's manifest: what the packer records about one image and what
 * the worker host requires before it wraps a module or boots the tree. Both
 * sides use these types; the worker validates the fetched file with
 * {@link parseImageManifest}.
 */
import { LOWERING_VERSION } from "./image-layout.js";
function recordOf(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value)
        ? value
        : undefined;
}
/** A relative path made of non-empty segments without traversal. */
function isRelativePath(value) {
    return typeof value === 'string' && value !== '' && !value.startsWith('/')
        && value.split('/').every(segment => segment !== '' && segment !== '.' && segment !== '..');
}
function isCount(value) {
    return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
/**
 * Validate a fetched base-image manifest.
 * @param value - Parsed manifest JSON.
 * @param path - Manifest path inside the image, named in refusals.
 * @returns The detached manifest.
 * @throws When the manifest is malformed, or its bodies were lowered against another wrapper contract.
 */
export function parseImageManifest(value, path) {
    const record = recordOf(value);
    if (record === undefined)
        throw new Error(`webworker host: ${path} does not hold an object`);
    if (record.lowered !== LOWERING_VERSION) {
        throw new Error(`webworker host: image was lowered by ${String(record.lowered)}, this build runs ${LOWERING_VERSION}; rebuild the image`);
    }
    const { root, profile, installation, javascriptEntries, visitedEntries, rewrittenEntries, lazy } = record;
    if (typeof root !== 'string' || typeof profile !== 'string' || profile === ''
        || (installation !== undefined && (typeof installation !== 'string' || installation === ''))
        || !isCount(javascriptEntries) || !isCount(visitedEntries) || !isCount(rewrittenEntries) || !Array.isArray(lazy)) {
        throw new Error(`webworker host: ${path} lacks a valid root, profile, installation, entry count, or lazy archive list`);
    }
    const archives = [];
    const archiveNames = new Set();
    const filePaths = new Set();
    for (const entry of lazy) {
        const archive = recordOf(entry);
        const name = archive?.archive;
        const files = archive?.files;
        if (!isRelativePath(name) || archiveNames.has(name) || !Array.isArray(files) || files.length === 0) {
            throw new Error(`webworker host: ${path} lists an invalid or repeated lazy archive`);
        }
        archiveNames.add(name);
        const parsed = [];
        for (const candidate of files) {
            const file = recordOf(candidate);
            if (file === undefined || !isRelativePath(file.path) || !isCount(file.size) || filePaths.has(file.path)) {
                throw new Error(`webworker host: ${path} lists an invalid or repeated file in lazy archive ${name}`);
            }
            filePaths.add(file.path);
            parsed.push({ path: file.path, size: file.size });
        }
        archives.push({ archive: name, files: parsed });
    }
    return {
        root, profile, ...installation === undefined ? {} : { installation }, lowered: LOWERING_VERSION,
        javascriptEntries, visitedEntries, rewrittenEntries, lazy: archives,
    };
}
//# sourceMappingURL=image-manifest.js.map
/**
 * Host-filesystem implementation of `ctx.fs`. Realpath-derived target identity makes aliases
 * share stale guards, and writes through a symlink update its target without replacing the link.
 * @module @deepseek-ai/dsh-fs-local
 */
import { applyHunks } from '@deepseek-ai/dsh-fs-patch';
import { constants as bufferConstants } from 'node:buffer';
import { once } from 'node:events';
import { watch } from 'chokidar';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import z from '@deepseek-ai/schemastery';
import { FileSystem, FsError, FsVersion } from '@deepseek-ai/dsh-fs';
import { applyLiteralEdit, expandHomePrefix, listDirectory, localDisplayPath, normalizeLineEndings, probe, probeNoFollow, readForEdit, readByteWindow, readTextForDiff, readWholeBytes, readWholeText, resolveLocalTarget, restoreLineEndings, removeFile, streamWholeText, writeFileAtomic, } from "./fsio.js";
const DEFAULT_DIFF_BASIS_MAX_BYTES = 10 * 1024 * 1024;
const MAX_DIFF_BASIS_BYTES = Math.min(bufferConstants.MAX_LENGTH, bufferConstants.MAX_STRING_LENGTH);
/**
 * The host-filesystem backend. Reads resolve relative paths from {@link Config.cwd}
 * (a resolution default, NOT a containment boundary — see the filesystem
 * capability-seam Agent Note); enforce
 * containment with a stricter backend or a `tools/execute` permission plugin.
 */
export class LocalFileSystem extends FileSystem {
    async watch(target, changed, signal) {
        signal.throwIfAborted();
        const path = resolve(this.processPath(target));
        const directory = (await this.stat(target, signal))?.type === 'directory';
        signal.throwIfAborted();
        const root = directory ? path : dirname(path);
        const watcher = watch(root, {
            ignoreInitial: true, depth: 0,
            ignored: entry => !directory && resolve(entry) !== root && resolve(entry) !== path,
        });
        watcher.on('all', (_event, entry) => {
            if (directory || resolve(entry) === path)
                changed();
        });
        watcher.on('error', (error) => { changed(error instanceof Error ? error : new Error(String(error))); });
        try {
            await once(watcher, 'ready', { signal });
            return () => watcher.close();
        }
        catch (error) {
            await watcher.close();
            throw error;
        }
    }
    static Config = z.object({
        cwd: z.string().default(process.cwd()),
        diffBasisMaxBytes: z.number().default(DEFAULT_DIFF_BASIS_MAX_BYTES),
    });
    /** Validated config (schemastery applied the defaults before construction). */
    config;
    /** Test hook forwarded to fsio for atomic-publication boundaries. */
    internals = {};
    /** Per-targetKey tail promise: serializes mutating ops so the read→guard→write
     * window can't interleave, making concurrent writes/edits deterministically
     * ordered (one wins, the rest see the new version and reject as stale). */
    locks = new Map();
    constructor(ctx, config) {
        super(ctx);
        const resolved = config;
        if (!Number.isSafeInteger(resolved.diffBasisMaxBytes)
            || resolved.diffBasisMaxBytes <= 0
            || resolved.diffBasisMaxBytes > MAX_DIFF_BASIS_BYTES) {
            throw new Error(`fs-local: diffBasisMaxBytes must be a positive safe integer no greater than ${MAX_DIFF_BASIS_BYTES}`);
        }
        this.config = resolved;
    }
    /** Run one mutation under the same lock ordering as multi-target operations. */
    withLock(targetKey, op) {
        return this.withLocks([targetKey], op);
    }
    /** Acquire distinct keys in lexical order and release inner locks before outer locks. */
    withLocks(keys, op) {
        const ordered = [...new Set(keys)].sort();
        const acquire = async (index) => {
            const key = ordered[index];
            if (key === undefined)
                return op();
            const prior = this.locks.get(key) ?? Promise.resolve();
            const run = prior.then(() => acquire(index + 1));
            const tail = run.then(() => undefined, () => undefined);
            this.locks.set(key, tail);
            try {
                return await run;
            }
            finally {
                if (this.locks.get(key) === tail)
                    this.locks.delete(key);
            }
        };
        return acquire(0);
    }
    async resolve(path, opts) {
        if (opts?.signal?.aborted)
            throw new FsError('resolve aborted', 'FS_ABORTED');
        const local = await resolveLocalTarget(opts?.cwd ?? this.config.cwd, expandHomePrefix(path));
        if (opts?.signal?.aborted)
            throw new FsError('resolve aborted', 'FS_ABORTED');
        return { targetKey: local.targetKey, displayPath: local.displayPath };
    }
    processPath(target) {
        return String(target.targetKey);
    }
    processPathFromHostPath(hostPath) {
        return isAbsolute(hostPath) ? resolve(hostPath) : undefined;
    }
    fileUrl(target) {
        return pathToFileURL(this.processPath(target)).href;
    }
    contains(parent, child) {
        const path = relative(this.processPath(parent), this.processPath(child));
        return path === '' || (path !== '..' && !path.startsWith(`..${sep}`) && !isAbsolute(path));
    }
    async stat(target, signal) {
        if (signal?.aborted)
            throw new FsError('stat aborted', 'FS_ABORTED');
        const info = await probe(target.targetKey);
        if (signal?.aborted)
            throw new FsError('stat aborted', 'FS_ABORTED');
        if (!info)
            return undefined;
        return { version: info.version, type: info.type, size: info.size };
    }
    async lstat(path, opts, signal) {
        if (signal?.aborted)
            throw new FsError('lstat aborted', 'FS_ABORTED');
        if (path.trim().length === 0)
            throw new FsError('file_path must be a non-empty string', 'FS_NOT_FOUND');
        const cwd = opts?.cwd ?? this.config.cwd;
        const info = await probeNoFollow(localDisplayPath(cwd, expandHomePrefix(path)));
        if (signal?.aborted)
            throw new FsError('lstat aborted', 'FS_ABORTED');
        if (!info)
            return undefined;
        return { version: info.version, type: info.type, size: info.size };
    }
    async readText(target, signal) {
        return readWholeText({ displayPath: target.displayPath, targetKey: target.targetKey }, signal);
    }
    streamText(target, signal) {
        return Promise.resolve(streamWholeText({ displayPath: target.displayPath, targetKey: target.targetKey }, signal));
    }
    async readBytes(target, signal, maxBytes) {
        return readWholeBytes({ displayPath: target.displayPath, targetKey: target.targetKey }, signal, maxBytes, this.internals);
    }
    async readByteRange(target, range, signal) {
        return readByteWindow({ displayPath: target.displayPath, targetKey: target.targetKey }, range, signal);
    }
    async listDir(target, signal) {
        const entries = await listDirectory({ displayPath: target.displayPath, targetKey: target.targetKey }, signal);
        return entries.map(entry => ({
            name: entry.name,
            type: entry.type,
            target: { targetKey: entry.target.targetKey, displayPath: entry.target.displayPath },
            ...(entry.version !== undefined ? { version: entry.version } : {}),
            ...(entry.size !== undefined ? { size: entry.size } : {}),
        }));
    }
    async writeText(target, content, expected, signal) {
        return this.withLock(target.targetKey, async () => {
            const existing = await this.writeTarget(target, expected);
            // Capture an optional contextual-diff basis before the write. The bounded
            // reader checks the opened file itself, so an external replacement after
            // `probe()` cannot turn this best-effort presentation read into an
            // unbounded allocation. Either side at/above the configured limit yields
            // `before: null`; consumers retain their whole-file fallback.
            const diffable = existing !== null
                && Buffer.byteLength(content, 'utf8') < this.config.diffBasisMaxBytes;
            const before = diffable
                ? await readTextForDiff(target.targetKey, this.config.diffBasisMaxBytes, signal)
                : null;
            await writeFileAtomic(target.targetKey, content, existing?.mode, signal, this.internals, { displayPath: target.displayPath, createIfAbsent: expected?.kind === 'createIfAbsent' });
            const after = await probe(target.targetKey);
            return {
                operation: existing ? 'update' : 'create',
                version: this.versionAfterWrite(after, target),
                before,
                // LF-normalized to share the diff basis with `before` (also LF): a CRLF
                // overwrite must not read as every line changed. Line-ending restoration
                // is a storage detail the applied-hunk diff ignores.
                after: normalizeLineEndings(content),
            };
        });
    }
    async editText(target, edit, expected, signal) {
        return this.withLock(target.targetKey, async () => {
            const existing = await probe(target.targetKey);
            // Stale guard before literal matching: an edit based on an old read reports
            // FS_STALE_VERSION, not FS_EDIT_NOT_FOUND/FS_AMBIGUOUS_EDIT against newer content.
            // Missing targets use the same stale code on guarded and unconditional edit paths.
            if (!existing)
                throw new FsError(`cannot edit "${target.displayPath}": file changed since it was read`, 'FS_STALE_VERSION');
            if (existing.type !== 'file')
                throw new FsError(`cannot edit "${target.displayPath}": not a regular file`, 'FS_NOT_REGULAR_FILE');
            // expected === undefined: unconditional edit of the current content — no
            // version guard. Still inside the per-target lock, so the read→match→write
            // window is serialized and atomic.
            if (expected && existing.version !== expected.version) {
                throw new FsError(`cannot edit "${target.displayPath}": file changed since it was read`, 'FS_STALE_VERSION');
            }
            const original = await readForEdit(target.targetKey, target.displayPath, signal);
            const edited = applyLiteralEdit(original.content, edit.oldString, edit.newString, edit.replaceAll, target.displayPath);
            const content = restoreLineEndings(edited.content, original.lineEndings);
            await writeFileAtomic(target.targetKey, content, existing.mode, signal, this.internals, { displayPath: target.displayPath });
            const after = await probe(target.targetKey);
            return {
                version: this.versionAfterWrite(after, target),
                // The LF-normalized before/after text (the applied-hunk diff basis);
                // line-ending restoration is a storage detail the diff ignores.
                before: original.content,
                after: edited.content,
            };
        });
    }
    async patchText(target, hunks, expected, signal) {
        return this.withLock(target.targetKey, async () => {
            const existing = await this.requireFile(target, expected, 'patch');
            const original = await readForEdit(target.targetKey, target.displayPath, signal);
            const after = this.patchContent(target, original.content, hunks);
            await writeFileAtomic(target.targetKey, restoreLineEndings(after, original.lineEndings), existing.mode, signal, this.internals, { displayPath: target.displayPath });
            return { version: this.versionAfterWrite(await probe(target.targetKey), target), before: original.content, after };
        });
    }
    async deleteFile(target, expected, signal) {
        return this.withLock(target.targetKey, async () => {
            await this.requireFile(target, expected, 'delete');
            const before = await readTextForDiff(target.targetKey, this.config.diffBasisMaxBytes, signal);
            await this.internals.inspectBeforeMutation?.(target);
            if (signal?.aborted)
                throw new FsError('delete aborted', 'FS_ABORTED');
            await removeFile(target.targetKey, target.displayPath, this.internals);
            return { before };
        });
    }
    async moveFile(source, destination, options, signal) {
        if (source.targetKey === destination.targetKey) {
            throw new FsError(`cannot move "${source.displayPath}" to itself`, 'FS_SAME_TARGET');
        }
        return this.withLocks([source.targetKey, destination.targetKey], async () => {
            const sourceInfo = await this.requireFile(source, options.expectedSource, 'move');
            const existing = await this.writeTarget(destination, options.expectedDestination);
            const original = await readForEdit(source.targetKey, source.displayPath, signal);
            const after = options.hunks === undefined ? original.content : this.patchContent(source, original.content, options.hunks);
            const content = restoreLineEndings(after, original.lineEndings);
            const before = existing && Buffer.byteLength(content, 'utf8') < this.config.diffBasisMaxBytes
                ? await readTextForDiff(destination.targetKey, this.config.diffBasisMaxBytes, signal) : null;
            await this.internals.inspectBeforeMutation?.(source);
            if (signal?.aborted)
                throw new FsError('move aborted', 'FS_ABORTED');
            // After this checkpoint, complete both effects and report them even if cancellation arrives.
            await writeFileAtomic(destination.targetKey, content, sourceInfo.mode, undefined, this.internals, {
                displayPath: destination.displayPath,
                createIfAbsent: options.expectedDestination?.kind === 'createIfAbsent',
                existingDestination: existing !== null,
            });
            const outcome = {
                operation: existing ? 'update' : 'create',
                version: this.versionAfterWrite(await probe(destination.targetKey), destination), before, after,
            };
            const sourceBefore = Buffer.byteLength(original.content, 'utf8') < this.config.diffBasisMaxBytes ? original.content : null;
            try {
                await removeFile(source.targetKey, source.displayPath, this.internals);
            }
            catch (error) {
                // removeFile maps I/O failures; unexpected defects must still reject.
                if (!(error instanceof FsError))
                    throw error;
                return { sourceBefore, destination: outcome, sourceDeleted: false, sourceFailure: { code: error.code, message: error.message } };
            }
            return { sourceBefore, destination: outcome, sourceDeleted: true };
        });
    }
    async writeTarget(target, expected) {
        const existing = await probe(target.targetKey);
        if (existing && existing.type !== 'file') {
            throw new FsError(`cannot write "${target.displayPath}": not a regular file`, 'FS_NOT_REGULAR_FILE');
        }
        if (expected?.kind === 'replaceIfVersion') {
            // Stale guard: the file must still exist at the version the owner observed.
            if (!existing)
                throw new FsError(`cannot write "${target.displayPath}": file no longer exists`, 'FS_STALE_VERSION');
            if (existing.version !== expected.version) {
                throw new FsError(`cannot write "${target.displayPath}": file changed since it was read`, 'FS_STALE_VERSION');
            }
        }
        else if (expected?.kind === 'createIfAbsent' && existing) {
            // createIfAbsent onto an existing file: a blind overwrite — require a read first.
            throw new FsError(`cannot overwrite existing "${target.displayPath}" without reading it first`, 'FS_NOT_OBSERVED');
        }
        // No expectation means an unconditional but still atomic write.
        return existing;
    }
    async requireFile(target, expected, verb) {
        const existing = await probe(target.targetKey);
        if (!existing)
            throw new FsError(`cannot ${verb} "${target.displayPath}": file changed since it was read`, 'FS_STALE_VERSION');
        if (existing.type !== 'file')
            throw new FsError(`cannot ${verb} "${target.displayPath}": not a regular file`, 'FS_NOT_REGULAR_FILE');
        if (expected && existing.version !== expected.version) {
            throw new FsError(`cannot ${verb} "${target.displayPath}": file changed since it was read`, 'FS_STALE_VERSION');
        }
        return existing;
    }
    patchContent(target, before, hunks) {
        const result = applyHunks(before, hunks);
        if (!result.ok) {
            throw new FsError(`cannot patch "${target.displayPath}": hunk at patch line ${result.failure.hunkLineNumber} (${result.failure.stage}) does not match`, 'FS_PATCH_MISMATCH');
        }
        return result.after;
    }
    /* v8 ignore next 5 -- the post-write probe finding the file absent requires a
     * concurrent unlink between rename and stat; fall back to a sentinel version. */
    versionAfterWrite(after, target) {
        if (after)
            return after.version;
        return FsVersion(`missing:${target.targetKey}`);
    }
}
export default LocalFileSystem;
//# sourceMappingURL=index.js.map
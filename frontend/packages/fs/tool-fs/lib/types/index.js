/**
 * Model-facing read, read_image, write, edit, and apply_patch tools over `ctx.fs`. This package owns schemas, validation,
 * read windows, formatting, and observation events, never a concrete provider. An optional
 * event policy supplies mutation guards; without one the tools use unconditional provider calls.
 * @module @deepseek-ai/dsh-tool-fs
 */
import z from '@deepseek-ai/schemastery';
import { applyReadTool, READ_LIMIT, STREAM_MIN_SIZE } from "./read.js";
import { applyWriteTool } from "./write.js";
import { applyApplyPatchTool } from "./apply-patch.js";
import { applyEditTool } from "./edit.js";
import { applyReadImageTool } from "./read-image.js";
import { READ_MAX_BYTES, READ_MAX_LINE_LENGTH } from "./read-render.js";
import { FsSandboxController } from "./sandbox.js";
/** Cordis plugin name used by loader diagnostics. */
export const name = 'tool-fs';
/** Services required by the filesystem tool suite. */
export const inject = ['tools', 'fs', 'systemPrompt', 'workingDirectory'];
export const Config = z.object({
    read: z.boolean().default(true),
    readImage: z.boolean().default(true),
    write: z.boolean().default(true),
    edit: z.boolean().default(true),
    applyPatch: z.boolean().default(false),
    readLimit: z.number().default(READ_LIMIT),
    readMaxLineLength: z.number().default(READ_MAX_LINE_LENGTH),
    readMaxBytes: z.number().default(READ_MAX_BYTES),
    readStreamMinSize: z.number().default(STREAM_MIN_SIZE),
});
/** Every read cap counts lines/chars/bytes — a positive integer, or windowing arithmetic misbehaves silently. */
function assertPositiveInteger(name, value) {
    if (!Number.isInteger(value) || value < 1) {
        throw new Error(`tool-fs: ${name} must be a positive integer`);
    }
}
/** Register the selected filesystem tools; `read_image` additionally requires a mounted attachment store. */
export function apply(ctx, config) {
    // schemastery (Config) has already filled every defaulted field.
    const resolved = config;
    assertPositiveInteger('readLimit', resolved.readLimit);
    assertPositiveInteger('readMaxLineLength', resolved.readMaxLineLength);
    assertPositiveInteger('readMaxBytes', resolved.readMaxBytes);
    assertPositiveInteger('readStreamMinSize', resolved.readStreamMinSize);
    if (resolved.read) {
        applyReadTool(ctx, {
            limit: resolved.readLimit,
            maxLineLength: resolved.readMaxLineLength,
            maxBytes: resolved.readMaxBytes,
            streamMinSize: resolved.readStreamMinSize,
        });
    }
    // read_image is composition-conditional: without a mounted attachment store
    // the deployment cannot durably commit image bytes, so the tool never
    // registers; the execute body keeps a defensive re-check for direct callers.
    if (resolved.readImage) {
        ctx.inject(['attachments'], (imageCtx) => {
            applyReadImageTool(imageCtx);
        });
    }
    // One escalation API shared by the mutating tools: advertisement gating,
    // per-call policy resolution, and denial-marker mapping, all keyed off whether
    // the mounted ctx.fs confines (ctx.fs.sandboxMode).
    if (resolved.write || resolved.edit || resolved.applyPatch) {
        const sandbox = new FsSandboxController(ctx);
        if (resolved.write)
            applyWriteTool(ctx, sandbox);
        if (resolved.edit)
            applyEditTool(ctx, sandbox);
        if (resolved.applyPatch)
            applyApplyPatchTool(ctx, sandbox);
    }
}
//# sourceMappingURL=index.js.map
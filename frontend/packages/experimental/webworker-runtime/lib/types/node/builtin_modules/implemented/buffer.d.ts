/**
 * `node:buffer` for the worker, backed by the `buffer` npm package (feross), and
 * the matching `globalThis.Buffer` install. Node code treats Buffer as ambient,
 * so the global must exist before any host module evaluates.
 *
 * Loading this module replaces `Buffer.from`, `Buffer.prototype.toString`, and
 * `toLocaleString` on the package's class, so every Buffer user in the Worker,
 * including bundled third-party code, gets the same conversions. Whole-buffer
 * UTF-8 conversions use `TextEncoder` and `TextDecoder` with Node's U+FFFD
 * replacement and BOM retention; other encodings and ranged `toString` calls
 * keep the package's implementation.
 */
import { Buffer, kMaxLength } from 'buffer';
export { Buffer, kMaxLength };
/**
 * Size limits, as `node:buffer` publishes them. The npm package exposes only
 * `kMaxLength`, so the string bound is Node's own value for a 64-bit build.
 */
export declare const constants: {
    MAX_LENGTH: number;
    MAX_STRING_LENGTH: number;
};
/** CommonJS interop marker: the worker loader hands `default` to default imports (see ./builtins.ts). */
export declare const __esModule = true;
/** CommonJS default export: the members `require()` hands a caller of this module. */
declare const _default: {
    Buffer: BufferConstructor;
    constants: {
        MAX_LENGTH: number;
        MAX_STRING_LENGTH: number;
    };
    kMaxLength: number;
};
export default _default;
//# sourceMappingURL=buffer.d.ts.map
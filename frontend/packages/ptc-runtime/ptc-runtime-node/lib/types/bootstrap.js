/**
 * Output capture, result encoding, and host binding proxies over the process channel.
 * @module @deepseek-ai/dsh-ptc-runtime-node/src/bootstrap
 */
import { inspect, types } from 'node:util';
import { jsonStringBytesUpTo, jsonValueBytesUpTo, truncateJsonStringBytes } from "./output-json.js";
import { decodePtcJsonWire, encodePtcJsonWire, snapshotPtcJsonValue } from "./json-wire.js";
const CapturedError = Error;
const { isNativeError } = types;
const { create: capturedObjectCreate, defineProperty: capturedObjectDefineProperty, } = Object;
/** Define one public binding-error field without consulting mutable globals or descriptor prototypes. */
function defineBindingErrorField(error, key, value) {
    const attributes = capturedObjectCreate(null);
    attributes.enumerable = true;
    attributes.value = value;
    capturedObjectDefineProperty(error, key, attributes);
}
/**
 * Ordered text capture under the shared outer JSON-byte budget, delivered to
 * a sink as each item lands (the real sink sends text to the host eagerly,
 * so captured output survives a mid-run termination). It includes the log
 * array syntax and string escaping in its accounting. Once exhausted it emits
 * the fitting prefix and reports the limit once; the host turns that condition
 * into an explicit `output-limit` run failure.
 */
export class LogBuffer {
    bytes = 2; // JSON serialization of the empty logs array: []
    entries = 0;
    truncated = false;
    // Explicit fields, not constructor parameter properties: this module loads
    // under Node's native strip-only mode, which rejects non-erasable syntax —
    // and parameter properties are non-erasable.
    sink;
    onLimit;
    maxBytes;
    constructor(maxBytes, sink, onLimit = () => { }) {
        this.maxBytes = maxBytes;
        this.sink = sink;
        this.onLimit = onLimit;
    }
    /**
     * Emit text to the sink, charging it against the budget (drops + marks once exhausted).
     * @param text - the captured text to deliver.
     */
    push(text) {
        if (this.truncated)
            return;
        const separatorBytes = this.entries > 0 ? 1 : 0;
        const availableBytes = this.maxBytes - this.bytes - separatorBytes;
        const stringBytes = jsonStringBytesUpTo(text, availableBytes);
        if (stringBytes === undefined) {
            this.truncated = true;
            const prefix = truncateJsonStringBytes(text, availableBytes);
            if (prefix.length > 0) {
                const prefixBytes = jsonStringBytesUpTo(prefix, availableBytes);
                /* v8 ignore next -- truncateJsonStringBytes guarantees the returned prefix fits. */
                if (prefixBytes === undefined)
                    throw new CapturedError('program output ledger produced an oversized log prefix');
                this.bytes += prefixBytes + separatorBytes;
                this.entries += 1;
                this.sink(prefix);
            }
            this.onLimit();
            return;
        }
        this.bytes += stringBytes + separatorBytes;
        this.entries += 1;
        this.sink(text);
    }
    /** Whether captured text has exceeded the shared output budget. */
    get overflowed() { return this.truncated; }
    /** Remaining exact JSON-byte budget for the completion value or failure message. */
    remainingOutputBytes() {
        return this.maxBytes - this.bytes;
    }
}
/**
 * Render arguments without depth, array-length or string-length previews and emit registered images in order.
 * @param logs - the buffer every rendered line is pushed into.
 * @param images - Image identities owned by the execution process.
 * @param emitImage - Emits a registered image in the current evaluation.
 * @returns The program's inspect function and its matching Image collection hook.
 * @throws If object formatting fails; preceding arguments remain visible.
 */
export function makeInspect(logs, images, emitImage) {
    const collectors = new Map();
    const programInspect = (...args) => {
        const pending = [];
        const flush = () => {
            if (pending.length === 0)
                return;
            logs.push(pending.join(' ') + '\n');
            pending.length = 0;
        };
        for (const arg of args) {
            const directImage = arg !== null && typeof arg === 'object' ? images.get(arg) : undefined;
            if (directImage !== undefined) {
                flush();
                emitImage(directImage);
                continue;
            }
            const nested = [];
            const collector = Symbol();
            const options = {
                depth: null, maxArrayLength: null, maxStringLength: null,
                ptcImageCollector: collector,
            };
            collectors.set(collector, nested);
            try {
                pending.push(typeof arg === 'string' ? arg : inspect(arg, options));
            }
            catch (error) {
                flush();
                throw error;
            }
            finally {
                collectors.delete(collector);
            }
            if (nested.length === 0)
                continue;
            flush();
            for (const id of nested)
                emitImage(id);
        }
        flush();
        if (args.length === 0)
            logs.push('\n');
    };
    return {
        inspect: programInspect,
        collectImage(image, options) {
            const id = images.get(image);
            if (id !== undefined && options.ptcImageCollector !== undefined)
                collectors.get(options.ptcImageCollector)?.push(id);
        },
    };
}
/**
 * Redirect a stream's `write` into the log buffer (the program-visible
 * `process.stdout`/`process.stderr` in the Node child), so raw writes land in emission order
 * alongside console output instead of racing down a pipe. It preserves Node's optional callback
 * contract: the callback runs asynchronously after admission, even when the log budget drops
 * the write.
 *
 * @param logs - the buffer captured writes are pushed into.
 * @param stream - the stream whose `write` slot is patched.
 * @returns the restore function (the in-process tests un-patch; the real
 *   child never needs to).
 */
export function captureStreamWrites(logs, stream) {
    // The slot's VALUE is stored for restore and reassigned — never invoked
    // detached, so the unbound-method concern does not apply.
    // oxlint-disable-next-line typescript/unbound-method
    const original = stream.write;
    stream.write = (chunk, ...rest) => {
        logs.push(typeof chunk === 'string' ? chunk : String(chunk));
        // Node's optional-encoding shape: the callback is whichever of the next
        // two positions holds a function (a non-function there is the encoding).
        const callback = [rest[0], rest[1]].find((arg) => typeof arg === 'function');
        if (callback)
            queueMicrotask(() => { callback(null); });
        return true;
    };
    return () => { stream.write = original; };
}
/**
 * Prepare the program's completion value for the run result. Only lossless
 * JSON crosses, and a value that does not fit the remaining combined outer
 * budget reports `output-limit`; the host revalidates hostile traffic and
 * remains authoritative for native pipe writes the program shim cannot observe.
 *
 * @param value - the program's completion value.
 * @param remainingOutputBytes - exact bytes left after captured logs.
 * @param maxOutputBytes - the configured cap named in an overflow diagnostic.
 * @returns the result fragment: `{}` for `undefined`, else a flat wire `{ value }`.
 */
export function prepareCompletion(value, remainingOutputBytes, maxOutputBytes = remainingOutputBytes) {
    if (value === undefined)
        return {};
    let snapshot;
    try {
        snapshot = snapshotPtcJsonValue(value);
    }
    catch {
        snapshot = undefined;
    }
    if (snapshot === undefined) {
        return prepareFailure('invalid-output', 'program completion must be lossless JSON', remainingOutputBytes, maxOutputBytes);
    }
    if (jsonValueBytesUpTo(snapshot, remainingOutputBytes) === undefined) {
        return outputLimit(maxOutputBytes);
    }
    return { value: encodePtcJsonWire(snapshot) };
}
/**
 * Build the fixed overflow fragment without carrying rejected variable bytes.
 * @param maxOutputBytes Configured limit reported to the caller.
 * @returns The output-limit failure shared by evaluation phases.
 */
export function outputLimit(maxOutputBytes) {
    return { error: { kind: 'output-limit', message: `outer output exceeded ${maxOutputBytes} bytes` } };
}
/** Admit one bounded failure message or replace it with the fixed overflow diagnostic. */
function prepareFailure(kind, message, remainingOutputBytes, maxOutputBytes) {
    if (jsonStringBytesUpTo(message, remainingOutputBytes) === undefined)
        return outputLimit(maxOutputBytes);
    return { error: { kind, message } };
}
/**
 * Prepare a thrown program value, including cross-realm errors, without sending
 * an unbounded stack or string across the process channel.
 * @param error - the value thrown by the program.
 * @param remainingOutputBytes - exact bytes left after captured logs.
 * @param maxOutputBytes - the configured cap named in an overflow diagnostic.
 * @returns a bounded exception or fixed output-limit fragment.
 */
export function prepareException(error, remainingOutputBytes, maxOutputBytes = remainingOutputBytes) {
    let message;
    try {
        const detail = isNativeError(error) ? error.stack ?? error.message : error;
        message = typeof detail === 'string' ? detail : String(detail);
    }
    catch {
        message = 'program threw an unrenderable value';
    }
    return prepareFailure('exception', message, remainingOutputBytes, maxOutputBytes);
}
/**
 * Materialize the real error constructor declared by one namespace.
 * @param descriptor - program-global class name and member-name property.
 * @returns the constructor injected into the program and used for rejections.
 */
export function makeBindingErrorClass(descriptor) {
    return class BindingCallError extends CapturedError {
        constructor(memberName, message) {
            super(message);
            defineBindingErrorField(this, 'name', descriptor.name);
            defineBindingErrorField(this, descriptor.memberNameProperty, memberName);
        }
    };
}
/** Create the namespace-specific rejection for one failed binding call. */
function bindingFailure(errorClass, memberName, message) {
    return errorClass ? new errorClass(memberName, message) : new CapturedError(message);
}
/**
 * Build null-prototype binding globals with namespace-specific rejection classes.
 * @param data - Namespace declarations injected into the program.
 * @param host - Evaluation-scoped remote binding calls.
 * @param errorClasses - Constructors shared with program globals, resolved per invocation.
 * @param decodeImage - Optional decoder for host-issued images.
 * @returns One namespace object per declaration.
 */
export function makeNamespaces(data, host, errorClasses, decodeImage) {
    return data.namespaces.map(({ global, names }) => {
        const namespace = Object.create(null);
        for (const name of names) {
            Object.defineProperty(namespace, name, {
                enumerable: true,
                value: async (args) => {
                    const errorClass = errorClasses.get(global);
                    try {
                        let detached;
                        try {
                            detached = snapshotPtcJsonValue(args);
                        }
                        catch {
                            // Model-defined getters can fail while arguments are detached.
                            detached = undefined;
                        }
                        if (detached === undefined)
                            throw new CapturedError('binding arguments must be lossless JSON');
                        const result = decodePtcJsonWire(await host.invoke(global, name, encodePtcJsonWire(detached)), decodeImage);
                        if (result === undefined)
                            throw new CapturedError('binding resolution must be lossless JSON');
                        return result;
                    }
                    catch (error) {
                        throw bindingFailure(errorClass, name, error instanceof CapturedError ? error.message : String(error));
                    }
                },
            });
        }
        return namespace;
    });
}
//# sourceMappingURL=bootstrap.js.map
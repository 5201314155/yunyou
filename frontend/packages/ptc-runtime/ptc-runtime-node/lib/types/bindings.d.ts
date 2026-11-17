/** Validate and capture the binding names available to one program. */
import { PtcImage } from '@deepseek-ai/dsh-ptc-runtime';
import type { PtcBindingFunction, PtcBindingNamespace, PtcJsonValue, PtcRunRequest } from '@deepseek-ai/dsh-ptc-runtime';
import { type PtcJsonWire, type PtcImageWire } from './json-wire.ts';
/**
 * Reject unusable namespaces before starting a process.
 * @param request - Host-owned binding declarations.
 * @returns Namespaces keyed by their declared global.
 */
export declare function validateBindings(request: PtcRunRequest): Map<string, PtcBindingNamespace>;
/**
 * Invoke a host binding and encode its lossless result.
 * @param binding - Current evaluation's function, absent if it was removed.
 * @param args - Validated JSON arguments detached from the program.
 * @param image - Registers host-issued images with the owning process.
 * @returns Flat binding data; unavailable bindings and invalid results reject.
 */
export declare function invokeBinding(binding: PtcBindingFunction | undefined, args: PtcJsonValue, image?: (value: PtcImage) => PtcImageWire): Promise<PtcJsonWire>;
//# sourceMappingURL=bindings.d.ts.map
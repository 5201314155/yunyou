/** Combined byte accounting for retained program output. */
import type { PtcOutput, PtcJsonValue, PtcRunFailure, PtcRunResult } from '@deepseek-ai/dsh-ptc-runtime';
/** One run's combined outer-output ledger; binding values never enter it. */
export declare class OutputLedger {
    private readonly maxBytes;
    private readonly overflowMessage?;
    private bytes;
    private entries;
    constructor(maxBytes: number, overflowMessage?: string | undefined);
    /** Admit one ordered text or image entry against the combined output budget. */
    admit(entry: PtcOutput, sink: PtcOutput[]): boolean;
    /** Finalize a successful absent-or-JSON completion against the combined cap. */
    success(output: PtcOutput[], value?: PtcJsonValue): PtcRunResult;
    /** Finalize a failure diagnostic, with output-limit taking precedence when combined bytes exceed the cap. */
    failure(output: PtcOutput[], error: PtcRunFailure): PtcRunResult;
    /** Build the explicit output-limit failure while retaining a fitting ordered output prefix. */
    limit(output: PtcOutput[]): PtcRunResult;
}
//# sourceMappingURL=output-ledger.d.ts.map
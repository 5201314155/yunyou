/** Pure configuration changes for Node PTC profile editors. */
import type { PtcProgramFormat } from '@deepseek-ai/dsh-ptc-runtime';
import type { Config } from './config.ts';
/**
 * Select retained Node execution while preserving Script and scoped REPL syntax.
 * @param programFormat - Effective syntax; omission uses the Node provider's default function-body syntax.
 * @returns All three settings to merge into the profile atomically, replacing stale values; function bodies and REPL statements
 * switch to `persistent-in-memory` with `SCOPED_REPL_BY_DEFAULT`, which is also the default when `replScoped` is omitted.
 */
export declare function resolvePersistentExecution(programFormat?: PtcProgramFormat): Pick<Required<Config>, 'executionMode' | 'scriptPersistence' | 'replScoped'>;
//# sourceMappingURL=configuration.d.ts.map
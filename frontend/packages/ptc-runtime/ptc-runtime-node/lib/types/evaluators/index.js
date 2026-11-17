import { createAsyncFunctionEvaluator } from "./async-function.js";
import { createReplEvaluator } from "./repl.js";
import { createScopedReplEvaluator } from "./scoped-repl-evaluator.js";
import { createVmScriptEvaluator } from "./vm-script.js";
const evaluators = {
    'function-body': createAsyncFunctionEvaluator,
    repl: createReplEvaluator,
    script: createVmScriptEvaluator,
    'scoped-repl': createScopedReplEvaluator,
};
/**
 * Create the selected evaluator for either a fresh or retained process.
 * @param format - Evaluation syntax selected by the host.
 * @param reportError - Asynchronous diagnostics attributed to the active evaluation.
 * @param importHandler - Runner-local import handling; omission uses Node's loader.
 * @returns Globals, evaluation entry point, and resource cleanup.
 */
export function createProgramEvaluator(format, reportError, importHandler) {
    return evaluators[format]({ reportError, ...importHandler === undefined ? {} : { importHandler } });
}
//# sourceMappingURL=index.js.map
import { runNodeMain } from "./process.js";
/**
 * Serve evaluations through the installed retained-bootstrap entry.
 * @param stream - Inherited private control endpoint.
 * @param maxMessageBytes - Maximum control frame and queued output bytes.
 * @param processState - Native child process; structural test doubles do not replace Node's globals or built-in exports.
 * @param format - Default syntax for hosts that omit it from program data.
 * @param importHandler - Runner-local import decisions; omission retains native loading.
 * @returns After control closure and evaluator cleanup.
 */
export function runPersistentMain(stream, maxMessageBytes, processState, format = 'repl', importHandler) {
    return runNodeMain(stream, maxMessageBytes, processState, format, importHandler);
}
//# sourceMappingURL=persistent-process.js.map
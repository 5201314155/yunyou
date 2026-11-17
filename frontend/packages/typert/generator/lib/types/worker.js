/**
 * Worker Thread entry for workspace-mode Typert generation. It generates every
 * explicit contributor of the requested workspace, posts exactly one
 * {@link WorkspaceWorkerResponse}, and exits.
 * @module @deepseek-ai/dsh-typert-generator/worker
 */
import { parentPort, workerData } from 'node:worker_threads';
import { WorkspaceTypertGenerator } from "./workspace.js";
function parseRequest(value) {
    const request = typeof value === 'object' && value !== null
        ? value
        : {};
    const { root, faces, options } = request;
    const checkDiagnostics = typeof options === 'object' && options !== null
        ? options.checkDiagnostics
        : null;
    if (typeof root !== 'string'
        || (faces !== undefined
            && (!Array.isArray(faces) || !faces.every((face) => face === 'host' || face === 'client')))
        || (checkDiagnostics !== undefined && typeof checkDiagnostics !== 'boolean')) {
        throw new TypeError('typert-generator: malformed workspace worker request');
    }
    return {
        root,
        ...(faces === undefined ? {} : { faces }),
        options: checkDiagnostics === undefined ? {} : { checkDiagnostics },
    };
}
function respond() {
    try {
        const request = parseRequest(workerData);
        const generator = new WorkspaceTypertGenerator(request.root, request.options);
        return { kind: 'artifacts', artifacts: generator.generateContributors(request.faces) };
    }
    catch (error) {
        if (!(error instanceof Error))
            return { kind: 'failure', name: 'Error', message: String(error) };
        return {
            kind: 'failure',
            name: error.name,
            message: error.message,
            ...(error.stack === undefined ? {} : { stack: error.stack }),
        };
    }
}
if (parentPort === null)
    throw new Error('typert-generator: the workspace worker entry must run in a Worker Thread');
parentPort.postMessage(respond());
//# sourceMappingURL=worker.js.map
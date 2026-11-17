/**
 * Worker Thread entry for workspace-mode Typert generation. It generates every
 * explicit contributor of the requested workspace, posts exactly one
 * {@link WorkspaceWorkerResponse}, and exits.
 * @module @deepseek-ai/dsh-typert-generator/worker
 */
import type { WorkspaceEmitResult, WorkspaceTypertGeneratorOptions } from './workspace.ts';
import type { TypertFace } from './model.ts';
/** Generation inputs passed as `workerData`. */
export interface WorkspaceWorkerRequest {
    /** Workspace root containing the face aggregate tsconfigs. */
    readonly root: string;
    /** Faces to generate; every face when absent. */
    readonly faces?: readonly TypertFace[];
    /** Generator switches applied to the pass. */
    readonly options: WorkspaceTypertGeneratorOptions;
}
/** The single message the worker posts: every generated artifact, or the failure that stopped generation. */
export type WorkspaceWorkerResponse = {
    readonly kind: 'artifacts';
    readonly artifacts: readonly WorkspaceEmitResult[];
} | {
    readonly kind: 'failure';
    readonly name: string;
    readonly message: string;
    readonly stack?: string;
};
//# sourceMappingURL=worker.d.ts.map
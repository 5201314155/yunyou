/**
 * Optional tsdown (rolldown) plugin face of the typert generator. It lowers
 * standard decorators in TypeScript dependencies before bundling, then emits
 * model-driven face artifacts at the package output root. Packages without a
 * Typert or Remote export are skipped. Workspace mode generates in a Worker
 * Thread started by the pass's first `buildStart`, so bundling continues on the
 * main thread while Typert analyzes; the pass's first `writeBundle` awaits the
 * result and writes every artifact once.
 * @module @deepseek-ai/dsh-typert-generator/tsdown
 */
import type { TypertFace } from './model.ts';
/** The subset of the rolldown plugin contract used here (structural; avoids a rolldown type dependency). */
interface TypertPlugin {
    name: string;
    buildStart: (options: {
        cwd: string;
    }) => void;
    transform: {
        filter: {
            id: RegExp;
        };
        handler: (code: string, id: string) => {
            code: string;
            map: string | undefined;
        } | undefined;
    };
    writeBundle: (options: {
        dir?: string;
    }) => Promise<void>;
}
/** Generation scope selected by a tsdown build phase. */
export interface TypertPluginOptions {
    /** Package mode emits only the package being bundled; workspace mode emits every explicit contributor once. */
    readonly mode?: 'package' | 'workspace';
    /** Independent TypeScript program faces included in this phase. */
    readonly faces?: readonly TypertFace[];
}
/** A workspace-mode generation failure reported by, or observed on, the Typert Worker Thread. */
export declare class TypertWorkerError extends Error {
    name: string;
}
/**
 * Lower standard TypeScript decorators without reading or writing metadata.
 * @returns a rolldown-compatible transform without metadata generation or artifact writes.
 */
export declare function decoratorPlugin(): Pick<TypertPlugin, 'name' | 'transform'>;
/**
 * Create the decorator-lowering and Typert-generation plugin for generation during bundling.
 * @param pluginOptions - package/workspace emission mode and independent program faces.
 * @returns a rolldown-compatible plugin that lowers source decorators and emits local and Host-for-Client artifacts.
 */
export declare function typertPlugin(pluginOptions?: TypertPluginOptions): TypertPlugin;
export {};
//# sourceMappingURL=tsdown-plugin.d.ts.map
/**
 * Local-filesystem implementation of `ctx.fileReferences`.
 *
 * @module @deepseek-ai/dsh-file-reference-local
 */
import { Context } from '@deepseek-ai/cordis';
import z from '@deepseek-ai/schemastery';
import type { Agent } from '@deepseek-ai/dsh-agent';
import FileReferenceService, { type FileReferenceCandidate } from '@deepseek-ai/dsh-file-reference';
export type { FileSearchConfig } from './search.ts';
/** Local file-reference discovery configuration. */
export interface Config {
    /** Maximum ranked candidates returned for one query. */
    maxResults?: number;
    /** Maximum indexed files and directories per agent workspace. */
    maxEntries?: number;
    /** Directory basenames never traversed or offered. */
    excludedDirectories?: string[];
}
/** Local-filesystem owner of the file-reference discovery service. */
export declare class LocalFileReferenceService extends FileReferenceService {
    static inject: string[];
    static Config: z<Config>;
    private readonly config;
    private readonly searches;
    private readonly promptFibers;
    private readonly promptDisposals;
    constructor(ctx: Context, config?: Config);
    list(agent: Agent, query: string, signal: AbortSignal): Promise<FileReferenceCandidate[]>;
}
export default LocalFileReferenceService;
//# sourceMappingURL=index.d.ts.map
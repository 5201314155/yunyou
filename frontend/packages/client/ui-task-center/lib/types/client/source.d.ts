/** 任务中心远程源：web.v1 yunyou/{tasks,schedule} 点形端点。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis';
import type { TaskCenterSource } from './contract.ts';
/**
 * 构建任务中心远程源（web.v1 真实 RPC）。
 * @param ctx - 客户端根上下文。
 * @returns 任务中心远程源。
 */
export declare function createTaskCenterSource(ctx: ClientContext): TaskCenterSource;
//# sourceMappingURL=source.d.ts.map
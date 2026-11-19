/** 团队远程源：web.v1 yunyou/{team,tasks} 点形端点（域细则 v1.2 终定）。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis';
import type { TeamSource } from './contract.ts';
/**
 * 构建团队远程源（web.v1 真实 RPC；错误码人话 message 恒中文，组件原样展示）。
 * @param ctx - 客户端根上下文。
 * @returns 团队远程源。
 */
export declare function createTeamSource(ctx: ClientContext): TeamSource;
//# sourceMappingURL=source.d.ts.map
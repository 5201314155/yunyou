/** 团队远程源：web.v1 yunyou/{team,tasks} 点形端点（域细则 v1.2 终定）。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis'
import type { ClientRemote } from '@deepseek-ai/dsh-api-remotes/client'
import type {} from '@deepseek-ai/dsh-yunyou-remotes/remote'
import type { TeamSource } from './contract.ts'

type YunyouRemote = ClientRemote['yunyou']

/** 把信封结果折叠为组件面 RemoteOutcome（业务成败在 result 内，规格 §2.1）。 */
async function call<T>(promise: Promise<{ ok: true; value: T } | { ok: false; error: { code: string; message: string } }>): Promise<
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly code: string; readonly message: string }
> {
  const result = await promise
  return result.ok
    ? { ok: true, value: result.value }
    : { ok: false, code: result.error.code, message: result.error.message }
}

/**
 * 构建团队远程源（web.v1 真实 RPC；错误码人话 message 恒中文，组件原样展示）。
 * @param ctx - 客户端根上下文。
 * @returns 团队远程源。
 */
export function createTeamSource(ctx: ClientContext): TeamSource {
  const remote: YunyouRemote = ctx.remote.yunyou
  return {
    roster: () => call(remote['team.roster']()),
    tasks: () => call(remote['tasks.list']()),
    spawn: request => call(remote['team.spawn'](request)),
    message: request => call(remote['team.message'](request)),
    kick: request => call(remote['team.kick'](request)),
    claim: (taskId, owner) => call(remote['tasks.claim']({ taskId, ...(owner === undefined ? {} : { owner }) })),
    submit: taskId => call(remote['tasks.submit']({ taskId })),
    review: (taskId, approve) => call(remote['tasks.review']({ taskId, approve })),
    reassign: (taskId, newOwner) => call(remote['tasks.reassign']({ taskId, newOwner })),
  }
}

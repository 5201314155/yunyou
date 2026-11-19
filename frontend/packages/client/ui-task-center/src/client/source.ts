/** 任务中心远程源：web.v1 yunyou/{tasks,schedule} 点形端点。 */
import type { Context as ClientContext } from '@deepseek-ai/cordis'
import type { ClientRemote } from '@deepseek-ai/dsh-api-remotes/client'
import type {} from '@deepseek-ai/dsh-yunyou-remotes/remote'
import type { TaskCenterSource } from './contract.ts'

/**
 * 构建任务中心远程源（web.v1 真实 RPC）。
 * @param ctx - 客户端根上下文。
 * @returns 任务中心远程源。
 */
export function createTaskCenterSource(ctx: ClientContext): TaskCenterSource {
  const remote: ClientRemote['yunyou'] = ctx.remote.yunyou
  const fold = async <T>(promise: Promise<{ ok: true; value: T } | { ok: false; error: { code: string; message: string } }>) => {
    const result = await promise
    return result.ok
      ? { ok: true as const, value: result.value }
      : { ok: false as const, code: result.error.code, message: result.error.message }
  }
  return {
    tasks: () => fold(remote['tasks.list']()),
    scheduleList: () => fold(remote['schedule.list']()),
    scheduleCreate: request => fold(remote['schedule.create'](request)),
    scheduleDelete: id => fold(remote['schedule.delete']({ id })),
  }
}

/** 团队远程源：阶段 1 诚实占位；阶段 2 接 web.v1 yunyou/{team,tasks} 真实 RPC 面。 */
import type { TeamSource } from './contract.ts'

/** 统一占位回执（fail-visible，不伪造数据；阶段 2 整体替换本文件实现）。 */
function notImplemented(): { readonly ok: false; readonly code: string; readonly message: string } {
  return { ok: false, code: 'gateway/not-implemented', message: 'team/tasks Web 通道未开通' }
}

/** 阶段 1 占位源。 */
export const stage1TeamSource: TeamSource = {
  roster: () => Promise.resolve(notImplemented()),
  tasks: () => Promise.resolve(notImplemented()),
  spawn: () => Promise.resolve(notImplemented()),
  message: () => Promise.resolve(notImplemented()),
  kick: () => Promise.resolve(notImplemented()),
  claim: () => Promise.resolve(notImplemented()),
  submit: () => Promise.resolve(notImplemented()),
  review: () => Promise.resolve(notImplemented()),
  reassign: () => Promise.resolve(notImplemented()),
}

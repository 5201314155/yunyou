/** 团队中心数据形状与远程面接口缝（web.v1 team/tasks 域，域细则 §1–§2）。 */

/** 名册行（team/roster，L1）：online/busy/task 由服务端合成，前端只读。 */
export interface RosterEntry {
  readonly name: string
  readonly description: string
  readonly online: boolean
  readonly busy: boolean
  readonly task?: string
}

/** 共享任务状态机（board.rs 冻结）：rejected 为终态，重做=新建。 */
export const TASK_STATES = ['pending', 'claimed', 'submitted', 'reviewing', 'approved', 'rejected'] as const
export type TaskState = (typeof TASK_STATES)[number]

/** 任务板行（tasks/list 全量投影，L1）。 */
export interface TaskEntry {
  readonly id: string
  readonly title: string
  readonly state: TaskState
  readonly owner?: string
  readonly createdBy: string
  readonly submittedBy?: string
  readonly reviewer?: string
  readonly capability?: string
  readonly blockedBy: readonly string[]
}

/** 加载呈现阶段；error 携带人话 message（fail-visible，不伪装成功）。 */
export type LoadPhase = 'idle' | 'loading' | 'ready' | 'error'

/** 统一远程结果：ok=false 携稳定错误码（供诚实空态/错误提示分派）。 */
export type RemoteOutcome<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly code: string; readonly message: string }

/** 写审操作请求形状（契约 §1.2–§2.2，逐字）。 */
export interface SpawnRequest {
  readonly name: string
  readonly description: string
  readonly prompt: string
  readonly context: 'fresh' | 'fork'
}
export interface MessageRequest {
  readonly to: string
  readonly body: string
  readonly kind?: 'chat' | 'report'
}

/**
 * 团队远程面接口缝：阶段 1 由宿主注入诚实占位实现（恒 not-implemented），
 * 阶段 2 接 web.v1 真实 RPC（yunyou/team・tasks 命名空间，候云络别名）。
 * 组件只依赖此接口，不接触传输层。
 */
export interface TeamSource {
  readonly roster: () => Promise<RemoteOutcome<readonly RosterEntry[]>>
  readonly tasks: () => Promise<RemoteOutcome<readonly TaskEntry[]>>
  readonly spawn: (request: SpawnRequest) => Promise<RemoteOutcome<{ readonly name: string }>>
  readonly message: (request: MessageRequest) => Promise<RemoteOutcome<{ readonly accepted: true }>>
  readonly kick: (name: string) => Promise<RemoteOutcome<{ readonly kicked: true }>>
  readonly claim: (taskId: string, owner?: string) => Promise<RemoteOutcome<{ readonly claimed: true }>>
  readonly submit: (taskId: string) => Promise<RemoteOutcome<{ readonly submitted: true }>>
  readonly review: (taskId: string, approve: boolean) => Promise<RemoteOutcome<{ readonly approved: boolean }>>
  readonly reassign: (taskId: string, newOwner: string) => Promise<RemoteOutcome<{ readonly reassigned: true }>>
}

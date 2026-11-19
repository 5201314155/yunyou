/** 任务中心数据形状与远程面接口缝（web.v1 tasks/schedule 域）。 */
import type { ScheduleListEntry, ScheduleSpec, TasksListEntry } from '@deepseek-ai/dsh-yunyou-remotes/types'

/** 任务板行（tasks/list 全量投影）。 */
export type TaskEntry = TasksListEntry
export type { ScheduleListEntry, ScheduleSpec }

/** 加载呈现阶段。 */
export type LoadPhase = 'idle' | 'loading' | 'ready' | 'error'

/** 统一远程结果。 */
export type RemoteOutcome<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly code: string; readonly message: string }

/** 定时创建请求（三选一必填恰一；durable 语义=会话内有效）。 */
export interface ScheduleCreateRequest {
  readonly at_ms?: number
  readonly every_minutes?: number
  readonly cron?: string
  readonly message: string
  readonly durable?: boolean
}

/** 任务中心远程面接口缝（组件只依赖此接口）。 */
export interface TaskCenterSource {
  readonly tasks: () => Promise<RemoteOutcome<{ readonly items: readonly TaskEntry[] }>>
  readonly scheduleList: () => Promise<RemoteOutcome<ScheduleListValueShape>>
  readonly scheduleCreate: (request: ScheduleCreateRequest) => Promise<
    RemoteOutcome<{ readonly id: string; readonly nextFireMs: number; readonly nowMs: number; readonly message: string; readonly note: string }>
  >
  readonly scheduleDelete: (id: string) => Promise<RemoteOutcome<{ readonly deleted: string }>>
}

/** schedule/list 值形状（避免类型跨域再导出别名）。 */
export interface ScheduleListValueShape {
  readonly entries: readonly ScheduleListEntry[]
  readonly nowMs: number
}

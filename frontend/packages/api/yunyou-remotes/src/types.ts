/** web.v1 yunyou/{team,tasks,schedule} 域线形状（openapi x-method 逐字，规格 §2.2 错误码词表）。 */

/** 名册行（team/roster）：online/busy/task 服务端合成。 */
export interface TeamRosterEntry {
  readonly name: string
  readonly description: string
  readonly online: boolean
  readonly busy: boolean
  readonly task?: string
}

/** team/roster 值。 */
export interface TeamRosterValue {
  readonly items: readonly TeamRosterEntry[]
}

/** team/spawn 请求（L4，显式授权；context=fork 为摘要降级形态）。 */
export interface TeamSpawnRequest {
  readonly name: string
  readonly description: string
  readonly prompt: string
  readonly context: 'fresh' | 'fork'
}

/** team/spawn 值。 */
export interface TeamSpawnValue {
  readonly name: string
  readonly description: string
  readonly note: string
}

/** team/message 请求（L2，身份=lead 服务端强制）。 */
export interface TeamMessageRequest {
  readonly to: string
  readonly body: string
  readonly kind?: 'chat' | 'report'
}

/** team/message 值（SentReceipt 投影）。 */
export interface TeamMessageValue {
  readonly to: string
  readonly accepted: true
  readonly logId: string
}

/** team/kick 请求（L3）。 */
export interface TeamKickRequest {
  readonly name: string
}

/** team/kick 值。 */
export interface TeamKickValue {
  readonly kicked: true
}

/** tasks/list 行（全量投影）。 */
export interface TasksListEntry {
  readonly id: string
  readonly title: string
  readonly state: 'pending' | 'claimed' | 'submitted' | 'reviewing' | 'approved' | 'rejected'
  readonly owner?: string
  readonly createdBy: string
  readonly submittedBy?: string
  readonly reviewer?: string
  readonly capability?: string
  readonly blockedBy: readonly string[]
}

/** tasks/list 值。 */
export interface TasksListValue {
  readonly items: readonly TasksListEntry[]
}

/** tasks/create 请求。 */
export interface TasksCreateRequest {
  readonly title: string
  readonly description: string
  readonly capability?: string
  readonly blocked_by?: readonly string[]
}

/** tasks/create 值。 */
export interface TasksCreateValue {
  readonly taskId: string
}

/** tasks/claim 请求（owner 可代派）。 */
export interface TasksClaimRequest {
  readonly taskId: string
  readonly owner?: string
}

/** tasks/submit 请求。 */
export interface TasksSubmitRequest {
  readonly taskId: string
}

/** tasks/review 请求（L3，写审分离服务端强制）。 */
export interface TasksReviewRequest {
  readonly taskId: string
  readonly approve: boolean
}

/** tasks/review 值。 */
export interface TasksReviewValue {
  readonly approved: boolean
}

/** tasks/reassign 请求（L3）。 */
export interface TasksReassignRequest {
  readonly taskId: string
  readonly newOwner: string
}

/** schedule/create 请求（三选一必填恰一；durable 语义=会话内有效）。 */
export interface ScheduleCreateRequest {
  readonly at_ms?: number
  readonly every_minutes?: number
  readonly cron?: string
  readonly message: string
  readonly durable?: boolean
}

/** schedule/create 值（nextFireMs 回显即确认）。 */
export interface ScheduleCreateValue {
  readonly id: string
  readonly nextFireMs: number
  readonly nowMs: number
  readonly message: string
  readonly durable: boolean
  readonly note: string
}

/** schedule/list 行。 */
export interface ScheduleListEntry {
  readonly id: string
  readonly spec: Record<string, unknown>
  readonly nextFireMs: number
  readonly message: string
}

/** schedule/list 值。 */
export interface ScheduleListValue {
  readonly entries: readonly ScheduleListEntry[]
  readonly nowMs: number
}

/** schedule/delete 请求（L3）。 */
export interface ScheduleDeleteRequest {
  readonly id: string
}

/** schedule/delete 值。 */
export interface ScheduleDeleteValue {
  readonly deleted: string
}

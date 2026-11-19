/** web.v1 yunyou/{team,tasks,schedule} 域 Typert FaceModel（形状声明面）。 */
import type { Context } from '@deepseek-ai/cordis'
import { Remote, RemoteError, TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol'
import type {
  ScheduleCreateRequest, ScheduleCreateValue, ScheduleDeleteRequest, ScheduleDeleteValue,
  ScheduleListValue, TeamKickRequest, TeamKickValue, TeamMessageRequest, TeamMessageValue,
  TeamRosterValue, TeamSpawnRequest, TeamSpawnValue, TasksClaimRequest, TasksCreateRequest,
  TasksCreateValue, TasksListValue, TasksReassignRequest, TasksReviewRequest, TasksSubmitRequest,
} from './types.ts'

export type * from './types.ts'

/**
 * 形状声明面说明：yunyou-web（Rust 网关）是这些方法的真实实现（W3/W4 已接线，
 * 分派键=本面端点逐字）；本 TS 宿主不做业务实现，任何调用诚实返回
 * gateway/not-implemented（规格 §0.3 fail-visible，不伪造值）。
 */

/** 线形状声明占位：Rust 网关实现，本宿主恒 not-implemented。 */
function bridgeOnly(): never {
  throw new RemoteError('gateway/not-implemented', '该方法由 yunyou-web 网关桥接实现', {})
}

/** team 域 face：名册/拉人/成员消息/踢人。 */
export class YunyouTeamFace extends TypertRemoteService {
  /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
  constructor(ctx: Context) {
    super(ctx, 'yunyouTeam', { namespace: 'yunyou/team' })
  }

  /** 名册投影（L1）。 */
  @Remote
  roster(): Promise<TeamRosterValue> {
    return bridgeOnly()
  }

  /** 拉起成员（L4，需显式授权）。 */
  @Remote
  spawn(request: TeamSpawnRequest): Promise<TeamSpawnValue> {
    void request
    return bridgeOnly()
  }

  /** 成员消息（L2，身份=lead 服务端强制）。 */
  @Remote
  message(request: TeamMessageRequest): Promise<TeamMessageValue> {
    void request
    return bridgeOnly()
  }

  /** 踢人（L3）。 */
  @Remote
  kick(request: TeamKickRequest): Promise<TeamKickValue> {
    void request
    return bridgeOnly()
  }
}

/** tasks 域 face：共享任务板全量投影与状态机操作。 */
export class YunyouTasksFace extends TypertRemoteService {
  /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
  constructor(ctx: Context) {
    super(ctx, 'yunyouTasks', { namespace: 'yunyou/tasks' })
  }

  /** 任务板全量投影（L1）。 */
  @Remote
  list(): Promise<TasksListValue> {
    return bridgeOnly()
  }

  /** 创建任务（L2）。 */
  @Remote
  create(request: TasksCreateRequest): Promise<TasksCreateValue> {
    void request
    return bridgeOnly()
  }

  /** 认领（L2，owner 可代派）。 */
  @Remote
  claim(request: TasksClaimRequest): Promise<{ readonly claimed: true }> {
    void request
    return bridgeOnly()
  }

  /** 提交（L2，仅 owner）。 */
  @Remote
  submit(request: TasksSubmitRequest): Promise<{ readonly submitted: true }> {
    void request
    return bridgeOnly()
  }

  /** 审查裁决（L3，自审服务端拒绝）。 */
  @Remote
  review(request: TasksReviewRequest): Promise<TasksReviewValue> {
    void request
    return bridgeOnly()
  }

  /** 改派（L3）。 */
  @Remote
  reassign(request: TasksReassignRequest): Promise<{ readonly reassigned: true }> {
    void request
    return bridgeOnly()
  }
}

/** schedule 域 face：三选一定时任务（durable 语义=会话内有效）。 */
export class YunyouScheduleFace extends TypertRemoteService {
  /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
  constructor(ctx: Context) {
    super(ctx, 'yunyouSchedule', { namespace: 'yunyou/schedule' })
  }

  /** 创建（L2，nextFireMs 回显即确认）。 */
  @Remote
  create(request: ScheduleCreateRequest): Promise<ScheduleCreateValue> {
    void request
    return bridgeOnly()
  }

  /** 列表（L1）。 */
  @Remote
  list(): Promise<ScheduleListValue> {
    return bridgeOnly()
  }

  /** 删除（L3）。 */
  @Remote
  delete(request: ScheduleDeleteRequest): Promise<ScheduleDeleteValue> {
    void request
    return bridgeOnly()
  }
}

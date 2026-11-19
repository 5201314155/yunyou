/** web.v1 yunyou/{team,tasks,schedule} 域 Typert FaceModel（形状声明面）。 */
import type { Context } from '@deepseek-ai/cordis';
import { TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol';
import type { ScheduleCreateRequest, ScheduleCreateValue, ScheduleDeleteRequest, ScheduleDeleteValue, ScheduleListValue, TeamKickRequest, TeamKickValue, TeamMessageRequest, TeamMessageValue, TeamRosterValue, TeamSpawnRequest, TeamSpawnValue, TasksClaimRequest, TasksCreateRequest, TasksCreateValue, TasksListValue, TasksReassignRequest, TasksReviewRequest, TasksReviewValue, TasksSubmitRequest } from './types.ts';
export type * from './types.ts';
/** team 域 face：名册/拉人/成员消息/踢人。 */
export declare class YunyouTeamFace extends TypertRemoteService {
    /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
    constructor(ctx: Context);
    /** 名册投影（L1）。 */
    roster(): Promise<TeamRosterValue>;
    /** 拉起成员（L4，需显式授权）。 */
    spawn(request: TeamSpawnRequest): Promise<TeamSpawnValue>;
    /** 成员消息（L2，身份=lead 服务端强制）。 */
    message(request: TeamMessageRequest): Promise<TeamMessageValue>;
    /** 踢人（L3）。 */
    kick(request: TeamKickRequest): Promise<TeamKickValue>;
}
/** tasks 域 face：共享任务板全量投影与状态机操作。 */
export declare class YunyouTasksFace extends TypertRemoteService {
    /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
    constructor(ctx: Context);
    /** 任务板全量投影（L1）。 */
    list(): Promise<TasksListValue>;
    /** 创建任务（L2）。 */
    create(request: TasksCreateRequest): Promise<TasksCreateValue>;
    /** 认领（L2，owner 可代派）。 */
    claim(request: TasksClaimRequest): Promise<{
        readonly claimed: true;
    }>;
    /** 提交（L2，仅 owner）。 */
    submit(request: TasksSubmitRequest): Promise<{
        readonly submitted: true;
    }>;
    /** 审查裁决（L3，自审服务端拒绝）。 */
    review(request: TasksReviewRequest): Promise<TasksReviewValue>;
    /** 改派（L3）。 */
    reassign(request: TasksReassignRequest): Promise<{
        readonly reassigned: true;
    }>;
}
/** schedule 域 face：三选一定时任务（durable 语义=会话内有效）。 */
export declare class YunyouScheduleFace extends TypertRemoteService {
    /** @param ctx - 宿主上下文（形状声明面无业务读取）。 */
    constructor(ctx: Context);
    /** 创建（L2，nextFireMs 回显即确认）。 */
    create(request: ScheduleCreateRequest): Promise<ScheduleCreateValue>;
    /** 列表（L1）。 */
    list(): Promise<ScheduleListValue>;
    /** 删除（L3）。 */
    delete(request: ScheduleDeleteRequest): Promise<ScheduleDeleteValue>;
}
//# sourceMappingURL=index.d.ts.map
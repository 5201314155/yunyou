/**
 * 构建任务中心远程源（web.v1 真实 RPC）。
 * @param ctx - 客户端根上下文。
 * @returns 任务中心远程源。
 */
export function createTaskCenterSource(ctx) {
    const remote = ctx.remote.yunyou;
    const fold = async (promise) => {
        const result = await promise;
        return result.ok
            ? { ok: true, value: result.value }
            : { ok: false, code: result.error.code, message: result.error.message };
    };
    return {
        tasks: () => fold(remote['tasks.list']()),
        scheduleList: () => fold(remote['schedule.list']()),
        scheduleCreate: request => fold(remote['schedule.create'](request)),
        scheduleDelete: id => fold(remote['schedule.delete']({ id })),
    };
}
//# sourceMappingURL=source.js.map
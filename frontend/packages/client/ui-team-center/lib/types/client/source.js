/** 把信封结果折叠为组件面 RemoteOutcome（业务成败在 result 内，规格 §2.1）。 */
async function call(promise) {
    const result = await promise;
    return result.ok
        ? { ok: true, value: result.value }
        : { ok: false, code: result.error.code, message: result.error.message };
}
/**
 * 构建团队远程源（web.v1 真实 RPC；错误码人话 message 恒中文，组件原样展示）。
 * @param ctx - 客户端根上下文。
 * @returns 团队远程源。
 */
export function createTeamSource(ctx) {
    const remote = ctx.remote.yunyou;
    return {
        roster: async () => {
            const result = await call(remote['team.roster']());
            return result.ok ? { ok: true, value: result.value.items } : result;
        },
        tasks: async () => {
            const result = await call(remote['tasks.list']());
            return result.ok ? { ok: true, value: result.value.items } : result;
        },
        spawn: request => call(remote['team.spawn'](request)),
        message: request => call(remote['team.message'](request)),
        kick: name => call(remote['team.kick']({ name })),
        claim: (taskId, owner) => call(remote['tasks.claim']({ taskId, ...(owner === undefined ? {} : { owner }) })),
        submit: taskId => call(remote['tasks.submit']({ taskId })),
        review: (taskId, approve) => call(remote['tasks.review']({ taskId, approve })),
        reassign: (taskId, newOwner) => call(remote['tasks.reassign']({ taskId, newOwner })),
    };
}
//# sourceMappingURL=source.js.map
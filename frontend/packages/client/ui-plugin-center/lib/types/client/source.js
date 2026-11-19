/** 统一占位回执（fail-visible；阶段 2 接 pluginMarket 服务四方法）。 */
function notImplemented() {
    return { ok: false, code: 'gateway/not-implemented', message: 'market Web 通道未开通' };
}
/** 阶段 1 占位源。 */
export const stage1MarketSource = {
    list: () => Promise.resolve(notImplemented()),
    detail: () => Promise.resolve(notImplemented()),
    install: () => Promise.resolve(notImplemented()),
};
//# sourceMappingURL=source.js.map
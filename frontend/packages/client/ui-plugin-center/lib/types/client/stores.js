/** 广场查看态：索引条目快照与阶段。 */
import { defineStore } from '@deepseek-ai/dsh-client-store';
const actions = {
    setPhase: (state, phase, entries) => {
        state.phase = phase;
        state.entries = entries;
    },
};
/**
 * 声明广场查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export function createMarketStore() {
    return defineStore({ init: () => ({ phase: 'idle', entries: [] }), actions });
}
//# sourceMappingURL=stores.js.map
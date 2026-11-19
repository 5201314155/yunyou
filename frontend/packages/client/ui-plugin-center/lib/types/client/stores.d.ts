/** 广场查看态：索引条目快照与阶段。 */
import { type EngineStoreHandle } from '@deepseek-ai/dsh-client-store';
import type { LoadPhase, MarketEntry } from './contract.ts';
export interface MarketData {
    phase: LoadPhase;
    entries: readonly MarketEntry[];
}
declare const actions: {
    setPhase: (state: MarketData, phase: LoadPhase, entries: readonly MarketEntry[]) => void;
};
/**
 * 声明广场查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export declare function createMarketStore(): EngineStoreHandle<MarketData, typeof actions>;
export type MarketStoreHandle = ReturnType<typeof createMarketStore>;
export {};
//# sourceMappingURL=stores.d.ts.map
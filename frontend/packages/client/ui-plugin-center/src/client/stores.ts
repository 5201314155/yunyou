/** 广场查看态：索引条目快照与阶段。 */
import { defineStore, type EngineStoreHandle } from '@deepseek-ai/dsh-client-store'
import type { LoadPhase, MarketEntry } from './contract.ts'

export interface MarketData {
  phase: LoadPhase
  entries: readonly MarketEntry[]
}

const actions = {
  setPhase: (state: MarketData, phase: LoadPhase, entries: readonly MarketEntry[]) => {
    state.phase = phase
    state.entries = entries
  },
}

/**
 * 声明广场查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export function createMarketStore(): EngineStoreHandle<MarketData, typeof actions> {
  return defineStore({ init: (): MarketData => ({ phase: 'idle', entries: [] }), actions })
}

export type MarketStoreHandle = ReturnType<typeof createMarketStore>

/** 广场远程源：阶段 1 诚实占位（market 契约=[后续]，不伪造索引与卡片）。 */
import type { MarketSource } from './contract.ts'

/** 统一占位回执（fail-visible；阶段 2 接 pluginMarket 服务四方法）。 */
function notImplemented(): { readonly ok: false; readonly code: string; readonly message: string } {
  return { ok: false, code: 'gateway/not-implemented', message: 'market Web 通道未开通' }
}

/** 阶段 1 占位源。 */
export const stage1MarketSource: MarketSource = {
  list: () => Promise.resolve(notImplemented()),
  detail: () => Promise.resolve(notImplemented()),
  install: () => Promise.resolve(notImplemented()),
}

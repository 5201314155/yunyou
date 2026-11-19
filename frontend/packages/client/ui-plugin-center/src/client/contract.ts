/** 插件广场数据形状与远程面接口缝（设计 §六；market/list・install 契约=[后续]）。 */

/** 加载呈现阶段。 */
export type LoadPhase = 'idle' | 'loading' | 'ready' | 'error'

/** 统一远程结果。 */
export type RemoteOutcome<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly code: string; readonly message: string }

/** 分类标签（§6.1 索引条目字段；空=未分类）。 */
export type PluginCategory = 'tools' | 'ui' | 'lifecycle' | 'documents' | 'uncategorized'

/** 广场条目（索引投影：名称/版本列表/分类/标签/兼容性声明）。 */
export interface MarketEntry {
  readonly id: string
  readonly name: string
  readonly description: string
  readonly category: PluginCategory
  readonly tags: readonly string[]
  readonly versions: readonly string[]
  /** engines.yunyou 声明（缺省=不设限）；不符=灰置给因（§6.2）。 */
  readonly enginesYunyou?: string
  readonly platformWeb: boolean
  /** 信任级（§6.4）：official/signature/unsigned。 */
  readonly trust: 'official' | 'signature' | 'unsigned'
}

/** 详情页三步文档序（§6.1：locale meta.detail→包 README→仓库 README）。 */
export interface MarketEntryDetail {
  readonly id: string
  readonly steps: ReadonlyArray<{ readonly title: string; readonly body: string }>
}

/**
 * 广场远程面接口缝（§6.1 pluginMarket 四方法映射；组件只依赖此接口）。
 * 阶段 1=诚实占位（恒 not-implemented）；服务端 market 域开闸后接真实面。
 */
export interface MarketSource {
  readonly list: () => Promise<RemoteOutcome<{ readonly items: readonly MarketEntry[] }>>
  readonly detail: (id: string) => Promise<RemoteOutcome<MarketEntryDetail>>
  /** 安装一律转发安装管线（§一.2 同管线律）；v1 恒 not-implemented=本地安装指引态。 */
  readonly install: (id: string, version: string) => Promise<RemoteOutcome<{ readonly installed: true }>>
}

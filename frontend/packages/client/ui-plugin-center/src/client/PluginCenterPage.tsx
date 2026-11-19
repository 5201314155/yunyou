/** 插件广场主面板：卡片列表+分类标签+搜索+详情三步文档序（设计 §六）。 */
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-layout/client'
import type { MarketEntry, PluginCategory } from './contract.ts'
import { PluginCard } from './PluginCard.tsx'
import { PluginDetail } from './PluginDetail.tsx'
import css from './PluginCenterPage.module.css'

/** 宿主注入面：远程源。 */
export interface PluginCenterInjected {
  readonly source: import('./contract.ts').MarketSource
}

/** 分类过滤键。 */
type CategoryFilter = PluginCategory | 'all'

/** 广场页 props（五份额派生）。 */
export type PluginCenterPageProps =
  PropsRuntime<'main'>
  & InjectFace<PluginCenterInjected>
  & PropsStore<import('./stores.ts').MarketStoreHandle>
  & PropsLocale<'pluginCenter'>

/**
 * 渲染插件广场：搜索+分类过滤+卡片网格；选中卡片展开三步详情。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 插件广场页。
 */
export function PluginCenterPage(props: PluginCenterPageProps) {
  const { useStore, actions, source, t } = props
  const entries = useStore(state => state.entries)
  const phase = useStore(state => state.phase)
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<CategoryFilter>('all')
  const [selected, setSelected] = useState<MarketEntry | undefined>(undefined)
  const refresh = (): void => {
    actions.setPhase('loading', [])
    void source.list().then(result => {
      if (result.ok) actions.setPhase('ready', result.value.items)
      else actions.setPhase('error', [])
    })
  }
  useEffect(() => { refresh() }, [refresh])
  const filtered = useMemo(() => entries.filter(entry => {
    if (category !== 'all' && entry.category !== category) return false
    const needle = query.trim().toLowerCase()
    if (needle === '') return true
    return entry.name.toLowerCase().includes(needle) || entry.description.toLowerCase().includes(needle)
  }), [entries, category, query])
  const categories: readonly CategoryFilter[] = ['all', 'tools', 'ui', 'lifecycle', 'documents', 'uncategorized']
  return (
    <div className={css.page}>
      <header className={css.header}>
        <h1 className={css.title}>{t('page.title')}</h1>
        <input
          className={css.search}
          value={query}
          onChange={event => { setQuery(event.target.value) }}
          placeholder={t('page.searchPlaceholder')}
          aria-label={t('page.searchPlaceholder')}
        />
        <Button variant="ghost" onClick={refresh}>{t('page.refresh')}</Button>
      </header>
      <div className={css.categories} role="tablist">
        {categories.map(name => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={category === name}
            className={css.category}
            onClick={() => { setCategory(name) }}
          >{t(`category.${name}` as const)}</button>
        ))}
      </div>
      {selected !== undefined ? (
        <PluginDetail t={t} entry={selected} source={source} onBack={() => { setSelected(undefined) }} />
      ) : phase === 'ready' && filtered.length === 0 ? (
        <p className={css.empty}>{t('list.empty')}</p>
      ) : phase === 'ready' ? (
        <div className={css.grid}>
          {filtered.map(entry => (
            <PluginCard key={entry.id} t={t} entry={entry} onSelect={() => { setSelected(entry) }} />
          ))}
        </div>
      ) : <p className={css.empty}>{t('page.unavailable')}</p>}
    </div>
  )
}

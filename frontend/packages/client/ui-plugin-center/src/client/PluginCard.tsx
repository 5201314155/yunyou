/** 广场卡片：信任徽标+分类+版本数+兼容灰置（§6.2/§6.4）。 */
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { MarketEntry } from './contract.ts'
import css from './PluginCard.module.css'

/** 卡片 props。 */
export interface PluginCardProps {
  readonly t: PropsLocale<'pluginCenter'>['t']
  readonly entry: MarketEntry
  readonly onSelect: () => void
}

/** 信任徽标键映射（§6.4 UI 呈现列）。 */
const TRUST_KEY = {
  official: 'card.official',
  signature: 'card.signed',
  unsigned: 'card.unsigned',
} as const

/**
 * 渲染广场卡片；版本兼容不符或无 Web 面时灰置并给因（不可点装）。
 * @param props - 条目与本地化。
 * @returns 广场卡片。
 */
export function PluginCard({ t, entry, onSelect }: PluginCardProps) {
  const enginesBlocked = entry.enginesYunyou !== undefined
  const platformBlocked = !entry.platformWeb
  const installable = !enginesBlocked && !platformBlocked
  return (
    <article className={css.card} data-installable={installable || undefined}>
      <div className={css.head}>
        <span className={css.name}>{entry.name}</span>
        <span className={css.trust} data-trust={entry.trust}>{t(TRUST_KEY[entry.trust])}</span>
      </div>
      <p className={css.description}>{entry.description}</p>
      <div className={css.meta}>
        <span className={css.versions}>{t('card.versions', { count: entry.versions.length })}</span>
        {entry.tags.map(tag => <span key={tag} className={css.tag}>{tag}</span>)}
      </div>
      {enginesBlocked ? (
        <p className={css.blocked}>{t('card.incompatible', { runtime: 'web', range: entry.enginesYunyou ?? '' })}</p>
      ) : platformBlocked ? <p className={css.blocked}>{t('card.platformMissing')}</p> : null}
      <div className={css.actions}>
        <Button variant="outline" onClick={onSelect}>{t('detail.back') === '' ? '' : entry.id}</Button>
        <Button variant="primary" disabled={!installable} onClick={onSelect}>{t('card.install')}</Button>
      </div>
    </article>
  )
}

/** 详情页三步文档序（§6.1：locale meta.detail→包 README→仓库 README）。 */
import { useEffect, useState } from 'react'
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { MarketEntry, MarketEntryDetail } from './contract.ts'
import css from './PluginDetail.module.css'

/** 详情 props。 */
export interface PluginDetailProps {
  readonly t: PropsLocale<'pluginCenter'>['t']
  readonly entry: MarketEntry
  readonly source: import('./contract.ts').MarketSource
  readonly onBack: () => void
}

/**
 * 渲染详情：拉取三步文档序步骤并按序呈现；失败保留返回路径（fail-visible）。
 * @param props - 条目、远程源与本地化。
 * @returns 详情面板。
 */
export function PluginDetail({ t, entry, source, onBack }: PluginDetailProps) {
  const [detail, setDetail] = useState<MarketEntryDetail | undefined>(undefined)
  const [error, setError] = useState<string | undefined>(undefined)
  useEffect(() => {
    let live = true
    void source.detail(entry.id).then(result => {
      if (!live) return
      if (result.ok) setDetail(result.value)
      else setError(`${result.code}: ${result.message}`)
    })
    return () => { live = false }
  }, [entry.id, source])
  return (
    <section className={css.detail} aria-label={entry.name}>
      <header className={css.head}>
        <Button variant="ghost" onClick={onBack}>{t('detail.back')}</Button>
        <h2 className={css.title}>{entry.name}</h2>
      </header>
      <h3 className={css.stepsTitle}>{t('detail.stepsTitle')}</h3>
      {error !== undefined ? <p className={css.error}>{t('opFailed', { message: error })}</p> : null}
      {detail !== undefined ? (
        <ol className={css.steps}>
          {detail.steps.map(step => (
            <li key={step.title} className={css.step}>
              <h4 className={css.stepTitle}>{step.title}</h4>
              <pre className={css.stepBody}>{step.body}</pre>
            </li>
          ))}
        </ol>
      ) : null}
      <p className={css.installNote}>{t('install.unavailable')}</p>
    </section>
  )
}

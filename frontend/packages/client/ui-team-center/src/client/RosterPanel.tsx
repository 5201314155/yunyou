/** 名册栏：成员在线/忙碌/当前任务与 L2–L3 操作入口。 */
import { useState } from 'react'
import { Button, StateDot, type StateDotState } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { LoadPhase, RosterEntry, TeamSource } from './contract.ts'
import { MemberDialogs } from './MemberDialogs.tsx'
import css from './RosterPanel.module.css'

/** 名册栏 props（纯数据+回调，见五份额纪律）。 */
export type RosterPanelProps = PropsLocale<'teamCenter'> & {
  readonly phase: LoadPhase
  readonly roster: readonly RosterEntry[]
  readonly onSpawn: () => void
  readonly source: TeamSource
  readonly notify: (text: string, level: 'info' | 'error') => void
}

/** 成员可用性点：busy 优先于 online（服务端合成语义）。 */
function availability(entry: RosterEntry): { state: StateDotState; key: 'roster.busy' | 'roster.online' | 'roster.offline' } {
  if (entry.busy) return { state: 'ongoing', key: 'roster.busy' }
  if (entry.online) return { state: 'done', key: 'roster.online' }
  return { state: 'idle', key: 'roster.offline' }
}

/**
 * 渲染名册栏：每行状态点+名字+当前任务；行内操作触发 L2/L3 确认流。
 * @param props - 快照、阶段、回调与本地化。
 * @returns 名册栏。
 */
export function RosterPanel({ t, phase, roster, onSpawn, source, notify }: RosterPanelProps) {
  const [dialog, setDialog] = useState<
    | { readonly kind: 'kick'; readonly name: string }
    | { readonly kind: 'message'; readonly name: string }
    | undefined
  >(undefined)
  const runKick = async (name: string): Promise<void> => {
    const result = await source.kick(name)
    if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
    else notify(`${t('roster.kick')} ${name}`, 'info')
  }
  const runMessage = async (name: string, body: string): Promise<void> => {
    const result = await source.message({ to: name, body })
    if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
    else notify(`${t('message.send')} → ${name}`, 'info')
  }
  return (
    <section className={css.panel} aria-label={t('roster.title')}>
      <header className={css.panelHeader}>
        <h2 className={css.panelTitle}>{t('roster.title')}</h2>
        <Button variant="ghost" onClick={onSpawn}>{t('roster.spawn')}</Button>
      </header>
      {phase === 'ready' && roster.length === 0 ? <p className={css.empty}>{t('roster.empty')}</p>
        : phase === 'ready' ? (
          <ul className={css.rows}>
            {roster.map(entry => {
              const availabilityState = availability(entry)
              return (
                <li key={entry.name} className={css.row}>
                  <span className={css.availability} data-state={availabilityState.state}>
                    <StateDot state={availabilityState.state} />
                    <span className={css.availabilityText}>{t(availabilityState.key)}</span>
                  </span>
                  <span className={css.name} title={entry.description}>{entry.name}</span>
                  <span className={css.task}>{entry.task ?? ''}</span>
                  <span className={css.rowActions}>
                    <Button variant="toolbar" onClick={() => { setDialog({ kind: 'message', name: entry.name }) }}>
                      {t('roster.message')}
                    </Button>
                    <Button variant="toolbar" onClick={() => { setDialog({ kind: 'kick', name: entry.name }) }}>
                      {t('roster.kick')}
                    </Button>
                  </span>
                </li>
              )
            })}
          </ul>
        ) : <p className={css.unavailable}>{t('page.unavailable')}</p>}
      <MemberDialogs
        t={t}
        dialog={dialog}
        onClose={() => { setDialog(undefined) }}
        onKick={runKick}
        onMessage={runMessage}
      />
    </section>
  )
}

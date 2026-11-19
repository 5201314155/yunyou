/** 定时计划列表：id/下次触发/消息三列与删除二次确认（L3）。 */
import { useState } from 'react'
import { Button, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { LoadPhase, ScheduleListEntry, TaskCenterSource } from './contract.ts'
import css from './ScheduleList.module.css'

/** 列表 props。 */
export type ScheduleListProps = PropsLocale<'taskCenter'> & {
  readonly phase: LoadPhase
  readonly entries: readonly ScheduleListEntry[]
  readonly source: TaskCenterSource
  readonly notify: (text: string, level: 'info' | 'error') => void
}

/**
 * 渲染定时计划列表；删除走 RiskConfirmation（契约 §3 L3）。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 定时计划列表。
 */
export function ScheduleList({ t, phase, entries, source, notify }: ScheduleListProps) {
  const [confirm, setConfirm] = useState<ScheduleListEntry | undefined>(undefined)
  const [acknowledged, setAcknowledged] = useState(false)
  const runDelete = (): void => {
    if (confirm === undefined) return
    void source.scheduleDelete(confirm.id).then(result => {
      if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
      else notify(`${t('schedule.delete')} · ${confirm.id}`, 'info')
      setConfirm(undefined)
      setAcknowledged(false)
    })
  }
  return (
    <div className={css.list}>
      {phase === 'ready' && entries.length === 0 ? <p className={css.empty}>{t('schedule.empty')}</p>
        : phase === 'ready' ? (
          <ul className={css.rows}>
            {entries.map(entry => (
              <li key={entry.id} className={css.row}>
                <span className={css.id} title={entry.id}>{entry.id}</span>
                <span className={css.next}>{new Date(entry.nextFireMs).toLocaleString()}</span>
                <span className={css.message} title={entry.message}>{entry.message}</span>
                <Button variant="toolbar" onClick={() => { setConfirm(entry); setAcknowledged(false) }}>
                  {t('schedule.delete')}
                </Button>
              </li>
            ))}
          </ul>
        ) : <p className={css.empty}>{t('page.unavailable')}</p>}
      {confirm !== undefined ? (
        <RiskConfirmation
          open
          title={t('schedule.deleteConfirm', { id: confirm.id })}
          description={t('schedule.durableNote')}
          acknowledgeLabel={t('schedule.deleteAck')}
          cancelLabel={t('confirm.cancel')}
          closeLabel={t('close')}
          confirmLabel={t('confirm.submit')}
          acknowledged={acknowledged}
          onAcknowledgedChange={setAcknowledged}
          onCancel={() => { setConfirm(undefined) }}
          onConfirm={runDelete}
        />
      ) : null}
    </div>
  )
}

/** 定时计划创建表单：三选一必填恰一+nextFireMs 回显确认（契约 §3）。 */
import { useState } from 'react'
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { ScheduleCreateRequest, TaskCenterSource } from './contract.ts'
import css from './ScheduleCreateForm.module.css'

type Kind = 'once' | 'interval' | 'cron'

/** 创建表单 props。 */
export interface ScheduleCreateFormProps {
  readonly t: PropsLocale<'taskCenter'>['t']
  readonly open: boolean
  readonly onClose: () => void
  readonly source: TaskCenterSource
  /** 创建成功后刷新列表（回显确认已含 nextFireMs）。 */
  readonly onCreated: () => void
}

/**
 * 渲染创建表单：单次/间隔/cron 三选一；提交即回显 nextFireMs（响应即确认步，
 * 服务端 nowMs+nextFireMs 为准；durable 无跨重启开关）。
 * @param props - 本地化、开合与远程源。
 * @returns 创建表单。
 */
export function ScheduleCreateForm({ t, open, onClose, source, onCreated }: ScheduleCreateFormProps) {
  const [kind, setKind] = useState<Kind>('once')
  const [atLocal, setAtLocal] = useState('')
  const [every, setEvery] = useState('5')
  const [cron, setCron] = useState('')
  const [message, setMessage] = useState('')
  const [pending, setPending] = useState(false)
  const close = (): void => { setPending(false); onClose() }
  if (!open) return null
  const request = (): ScheduleCreateRequest | undefined => {
    if (message.trim() === '') return undefined
    if (kind === 'once') {
      const at = Date.parse(atLocal)
      return Number.isNaN(at) ? undefined : { at_ms: at, message }
    }
    if (kind === 'interval') {
      const minutes = Number(every)
      return Number.isInteger(minutes) && minutes >= 5 ? { every_minutes: minutes, message } : undefined
    }
    return cron.trim() === '' ? undefined : { cron: cron.trim(), message }
  }
  return (
    <Modal open onClose={close} title={t('schedule.create')} closeLabel={t('close')} {...(css.sheet ? { className: css.sheet } : {})}>
      <form
        className={css.form}
        onSubmit={(event) => {
          event.preventDefault()
          const body = request()
          if (pending || body === undefined) return
          setPending(true)
          void source.scheduleCreate(body).then(result => {
            setPending(false)
            if (!result.ok) return
            onCreated()
            close()
          })
        }}
      >
        <fieldset className={css.field}>
          <legend className={css.fieldLabel}>{t('schedule.kind')}</legend>
          {(['once', 'interval', 'cron'] as const).map(name => (
            <label key={name} className={css.radio}>
              <input type="radio" name="schedule-kind" checked={kind === name} onChange={() => { setKind(name) }} />
              <span>{t(`schedule.kind.${name}` as const)}</span>
            </label>
          ))}
        </fieldset>
        {kind === 'once' ? (
          <label className={css.field}>
            <span className={css.fieldLabel}>{t('schedule.at')}</span>
            <input className={css.input} type="datetime-local" value={atLocal} onChange={e => { setAtLocal(e.target.value) }} required />
          </label>
        ) : null}
        {kind === 'interval' ? (
          <label className={css.field}>
            <span className={css.fieldLabel}>{t('schedule.every')}</span>
            <input className={css.input} type="number" min={5} step={1} value={every} onChange={e => { setEvery(e.target.value) }} required />
          </label>
        ) : null}
        {kind === 'cron' ? (
          <label className={css.field}>
            <span className={css.fieldLabel}>{t('schedule.cron')}</span>
            <input className={css.input} value={cron} onChange={e => { setCron(e.target.value) }} required />
          </label>
        ) : null}
        <label className={css.field}>
          <span className={css.fieldLabel}>{t('schedule.message')}</span>
          <textarea className={css.body} value={message} onChange={e => { setMessage(e.target.value) }} rows={3} required />
        </label>
        <p className={css.note}>{t('schedule.durableNote')}</p>
        <div className={css.actions}>
          <Button variant="ghost" type="button" onClick={close}>{t('schedule.cancel')}</Button>
          <Button variant="primary" type="submit" disabled={pending}>{t('schedule.confirm')}</Button>
        </div>
      </form>
    </Modal>
  )
}

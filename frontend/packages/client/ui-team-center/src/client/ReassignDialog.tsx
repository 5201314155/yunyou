/** 改派对话框（L3）：新负责人输入+风险确认（契约 tasks/reassign，owner 须在册）。 */
import { useState } from 'react'
import { Button, Modal, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { TaskEntry, TeamSource } from './contract.ts'
import css from './ReassignDialog.module.css'

/** 改派流程两步：录入负责人→风险确认。 */
export interface ReassignDialogProps {
  readonly t: PropsLocale<'teamCenter'>['t']
  readonly task: TaskEntry | undefined
  readonly rosterNames: readonly string[]
  readonly onClose: () => void
  readonly source: TeamSource
  readonly notify: (text: string, level: 'info' | 'error') => void
}

/**
 * 渲染改派流：输入新负责人（在册校验提示）→确认框列任务与目标→提交真实 RPC。
 * @param props - 本地化、目标任务、名册名单与远程源。
 * @returns 改派对话框（无任务时渲染 null）。
 */
export function ReassignDialog({ t, task, rosterNames, onClose, source, notify }: ReassignDialogProps) {
  const [owner, setOwner] = useState('')
  const [confirming, setConfirming] = useState(false)
  const [acknowledged, setAcknowledged] = useState(false)
  const [pending, setPending] = useState(false)
  const close = (): void => {
    setOwner(''); setConfirming(false); setAcknowledged(false); setPending(false)
    onClose()
  }
  if (task === undefined) return null
  if (!confirming) {
    return (
      <Modal open onClose={close} title={t('reassign.title', { title: task.title })} closeLabel={t('close')} {...(css.sheet ? { className: css.sheet } : {})}>
        <form
          className={css.form}
          onSubmit={(event) => {
            event.preventDefault()
            if (owner.trim() === '') return
            setConfirming(true)
          }}
        >
          <label className={css.field}>
            <span className={css.fieldLabel}>{t('reassign.owner')}</span>
            <input
              className={css.input}
              value={owner}
              onChange={event => { setOwner(event.target.value) }}
              list="team-center-roster-names"
              required
            />
            <datalist id="team-center-roster-names">
              {rosterNames.map(name => <option key={name} value={name} />)}
            </datalist>
          </label>
          <div className={css.actions}>
            <Button variant="ghost" type="button" onClick={close}>{t('confirm.cancel')}</Button>
            <Button variant="primary" type="submit" disabled={owner.trim() === ''}>{t('reassign.confirm')}</Button>
          </div>
        </form>
      </Modal>
    )
  }
  return (
    <RiskConfirmation
      open
      title={t('confirm.reassign.title', { title: task.title })}
      description={t('confirm.reassign.body', { owner: owner.trim() })}
      acknowledgeLabel={t('confirm.reassign.acknowledge')}
      cancelLabel={t('confirm.cancel')}
      closeLabel={t('close')}
      confirmLabel={t('confirm.submit')}
      acknowledged={acknowledged}
      disabled={pending}
      onAcknowledgedChange={setAcknowledged}
      onCancel={() => { setConfirming(false) }}
      onConfirm={() => {
        setPending(true)
        void source.reassign(task.id, owner.trim()).then(result => {
          if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
          else notify(`${t('reassign.confirm')} · ${task.title} → ${owner.trim()}`, 'info')
          close()
        })
      }}
    />
  )
}

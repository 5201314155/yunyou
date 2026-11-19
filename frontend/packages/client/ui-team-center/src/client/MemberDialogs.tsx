/** 成员操作对话框组：发消息（L2 直通）与踢人（L3 风险确认）。 */
import { useState } from 'react'
import { Button, Modal, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { TeamCenterKey } from './locales.ts'
import css from './MemberDialogs.module.css'

/** 对话框负荷：kick 走风险确认，message 走表单。 */
export type MemberDialog =
  | { readonly kind: 'kick'; readonly name: string }
  | { readonly kind: 'message'; readonly name: string }
  | undefined

/** 对话框组 props。 */
export interface MemberDialogsProps {
  readonly t: PropsLocale<'teamCenter'>['t']
  readonly dialog: MemberDialog
  readonly onClose: () => void
  readonly onKick: (name: string) => Promise<void>
  readonly onMessage: (name: string, body: string) => Promise<void>
}

/**
 * 渲染当前成员对话框；同一时刻至多一个。踢人的主操作在勾选确认前置不可用。
 * @param props - 本地化、当前对话框与回调。
 * @returns 对话框组（无对话框时渲染 null）。
 */
export function MemberDialogs({ t, dialog, onClose, onKick, onMessage }: MemberDialogsProps) {
  const [acknowledged, setAcknowledged] = useState(false)
  const [body, setBody] = useState('')
  const [pending, setPending] = useState(false)
  const close = (): void => {
    setAcknowledged(false)
    setBody('')
    setPending(false)
    onClose()
  }
  if (dialog === undefined) return null
  if (dialog.kind === 'kick') {
    return (
      <RiskConfirmation
        open
        title={t('confirm.kick.title', { name: dialog.name })}
        description={t('confirm.kick.body')}
        acknowledgeLabel={t('confirm.kick.acknowledge')}
        cancelLabel={t('confirm.cancel')}
        closeLabel={t('close')}
        confirmLabel={t('confirm.submit')}
        acknowledged={acknowledged}
        disabled={pending}
        onAcknowledgedChange={setAcknowledged}
        onCancel={close}
        onConfirm={() => {
          setPending(true)
          void onKick(dialog.name).then(close)
        }}
      />
    )
  }
  return (
    <Modal open onClose={close} title={t('message.title', { name: dialog.name })} closeLabel={t('close')} {...(css.sheet ? { className: css.sheet } : {})}>
      <form
        className={css.form}
        onSubmit={(event) => {
          event.preventDefault()
          if (body.trim() === '' || pending) return
          setPending(true)
          void onMessage(dialog.name, body).then(close)
        }}
      >
        <h2 className={css.title}>{t('message.body')}</h2>
        <label className={css.field}>
          <span className={css.fieldLabel}>{t('message.body')}</span>
          <textarea
            className={css.body}
            value={body}
            onChange={event => { setBody(event.target.value) }}
            rows={4}
            required
          />
        </label>
        <div className={css.actions}>
          <Button variant="ghost" type="button" onClick={close}>{t('message.cancel')}</Button>
          <Button variant="primary" type="submit" disabled={pending || body.trim() === ''}>{t('message.send')}</Button>
        </div>
      </form>
    </Modal>
  )
}

export type { TeamCenterKey }

/** 拉人对话框（L4，需用户显式确认提交；fork 降级语义如实说明）。 */
import { useState } from 'react'
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { TeamSource } from './contract.ts'
import css from './SpawnDialog.module.css'

/** 拉人对话框 props。 */
export interface SpawnDialogProps {
  readonly t: PropsLocale<'teamCenter'>['t']
  readonly open: boolean
  readonly onClose: () => void
  readonly source: TeamSource
  readonly notify: (text: string, level: 'info' | 'error') => void
}

/**
 * 渲染拉起成员表单：name/description/prompt/context 四字段；fork 附降级说明。
 * 提交走真实 spawn RPC（服务端重名 fail-loud 原样展示）。
 * @param props - 本地化、开合与远程源。
 * @returns 拉人对话框。
 */
export function SpawnDialog({ t, open, onClose, source, notify }: SpawnDialogProps) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [prompt, setPrompt] = useState('')
  const [context, setContext] = useState<'fresh' | 'fork'>('fresh')
  const [pending, setPending] = useState(false)
  const close = (): void => {
    setName(''); setDescription(''); setPrompt(''); setPending(false)
    onClose()
  }
  if (!open) return null
  return (
    <Modal open onClose={close} title={t('spawn.title')} closeLabel={t('close')} {...(css.sheet ? { className: css.sheet } : {})}>
      <form
        className={css.form}
        onSubmit={(event) => {
          event.preventDefault()
          if (pending || name.trim() === '' || prompt.trim() === '') return
          setPending(true)
          void source.spawn({ name: name.trim(), description: description.trim(), prompt, context }).then(result => {
            if (!result.ok) {
              setPending(false)
              notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
            } else close()
          })
        }}
      >
        <label className={css.field}>
          <span className={css.fieldLabel}>{t('spawn.name')}</span>
          <input className={css.input} value={name} onChange={event => { setName(event.target.value) }} required />
        </label>
        <label className={css.field}>
          <span className={css.fieldLabel}>{t('spawn.description')}</span>
          <input className={css.input} value={description} onChange={event => { setDescription(event.target.value) }} />
        </label>
        <label className={css.field}>
          <span className={css.fieldLabel}>{t('spawn.prompt')}</span>
          <textarea className={css.body} value={prompt} onChange={event => { setPrompt(event.target.value) }} rows={3} required />
        </label>
        <fieldset className={css.field}>
          <legend className={css.fieldLabel}>{t('spawn.context')}</legend>
          <label className={css.radio}>
            <input type="radio" name="spawn-context" checked={context === 'fresh'} onChange={() => { setContext('fresh') }} />
            <span>{t('spawn.context.fresh')}</span>
          </label>
          <label className={css.radio}>
            <input type="radio" name="spawn-context" checked={context === 'fork'} onChange={() => { setContext('fork') }} />
            <span>{t('spawn.context.fork')}</span>
          </label>
          {context === 'fork' ? <p className={css.forkNote}>{t('spawn.forkNote')}</p> : null}
        </fieldset>
        <div className={css.actions}>
          <Button variant="ghost" type="button" onClick={close}>{t('spawn.cancel')}</Button>
          <Button variant="primary" type="submit" disabled={pending}>{t('spawn.submit')}</Button>
        </div>
      </form>
    </Modal>
  )
}

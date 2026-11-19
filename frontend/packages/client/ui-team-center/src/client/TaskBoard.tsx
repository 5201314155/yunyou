/** 共享任务板：状态机行列表与 L2 写操作/L3 审查确认（写审分离面）。 */
import { useState } from 'react'
import { Button, RiskConfirmation } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsLocale } from '@deepseek-ai/dsh-client-ui-slots'
import type { LoadPhase, TaskEntry, TaskState, TeamSource } from './contract.ts'
import css from './TaskBoard.module.css'

type Translate = PropsLocale<'teamCenter'>['t']

/** 任务板 props。 */
export type TaskBoardProps = PropsLocale<'teamCenter'> & {
  readonly phase: LoadPhase
  readonly tasks: readonly TaskEntry[]
  readonly source: TeamSource
  readonly notify: (text: string, level: 'info' | 'error') => void
}

/** 状态徽标语义色（不只凭颜色传达：文案并行）。 */
const STATE_TONE: Readonly<Record<TaskState, 'idle' | 'ongoing' | 'done' | 'error'>> = {
  pending: 'idle',
  claimed: 'ongoing',
  submitted: 'ongoing',
  reviewing: 'ongoing',
  approved: 'done',
  rejected: 'error',
}

/** 行上允许的下一动作（状态机映射，契约 §2.1）。 */
function availableActions(task: TaskEntry): ReadonlyArray<'claim' | 'submit' | 'approve' | 'reject' | 'reassign'> {
  switch (task.state) {
    case 'pending': return ['claim']
    case 'claimed': return ['submit']
    case 'submitted':
    case 'reviewing': return ['approve', 'reject']
    default: return []
  }
}

/**
 * 渲染任务板：行内状态+负责人+阻塞来源；操作走真实 RPC，审查走风险确认。
 * @param props - 快照、阶段、远程源与本地化。
 * @returns 任务板。
 */
export function TaskBoard({ t, phase, tasks, source, notify }: TaskBoardProps) {
  const [confirm, setConfirm] = useState<
    | { readonly kind: 'review'; readonly task: TaskEntry; readonly approve: boolean }
    | undefined
  >(undefined)
  const [pending, setPending] = useState(false)
  const act = (task: TaskEntry, action: 'claim' | 'submit'): void => {
    setPending(true)
    const call = action === 'claim' ? source.claim(task.id) : source.submit(task.id)
    void call.then(result => {
      setPending(false)
      if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
      else notify(t(`task.${action}` as const) + ' · ' + task.title, 'info')
    })
  }
  const review = (task: TaskEntry, approve: boolean): void => {
    setPending(true)
    void source.review(task.id, approve).then(result => {
      setPending(false)
      if (!result.ok) notify(t('opFailed', { message: `${result.code}: ${result.message}` }), 'error')
      else notify(t(approve ? 'state.approved' : 'state.rejected') + ' · ' + task.title, 'info')
    })
  }
  return (
    <section className={css.panel} aria-label={t('tasks.title')}>
      <header className={css.panelHeader}>
        <h2 className={css.panelTitle}>{t('tasks.title')}</h2>
      </header>
      {phase === 'ready' && tasks.length === 0 ? <p className={css.empty}>{t('tasks.empty')}</p>
        : phase === 'ready' ? (
          <ul className={css.rows}>
            {tasks.map(task => (
              <TaskRowContent key={task.id} t={t} task={task} pending={pending}
                onAct={act} onReview={(approve) => { setConfirm({ kind: 'review', task, approve }) }} />
            ))}
          </ul>
        ) : <p className={css.unavailable}>{t('page.unavailable')}</p>}
      {confirm !== undefined ? (
        <RiskConfirmation
          open
          title={t('confirm.review.title', { title: confirm.task.title })}
          description={t('confirm.review.body')}
          acknowledgeLabel={t('confirm.review.acknowledge')}
          cancelLabel={t('confirm.cancel')}
          closeLabel={t('close')}
          confirmLabel={t('confirm.submit')}
          acknowledged={false}
          disabled={pending}
          onAcknowledgedChange={() => { setConfirm({ ...confirm }) }}
          onCancel={() => { setConfirm(undefined) }}
          onConfirm={() => { review(confirm.task, confirm.approve); setConfirm(undefined) }}
        />
      ) : null}
    </section>
  )
}

/** 单行内容（拆出保持页面组件行数纪律）。 */
function TaskRowContent({ t, task, pending, onAct, onReview }: {
  t: Translate
  task: TaskEntry
  pending: boolean
  onAct: (task: TaskEntry, action: 'claim' | 'submit') => void
  onReview: (approve: boolean) => void
}) {
  const actions = availableActions(task)
  const stateKey = `state.${task.state}` as const
  return (
    <li className={css.row} data-state={task.state}>
      <div className={css.rowMain}>
        <span className={css.stateBadge} data-tone={STATE_TONE[task.state]}>{t(stateKey)}</span>
        <span className={css.title} title={task.title}>{task.title}</span>
        <span className={css.meta}>
          {task.owner === undefined ? '' : `@${task.owner}`}
          {task.capability === undefined ? '' : ` · ${task.capability}`}
        </span>
        {task.blockedBy.length > 0 ? (
          <span className={css.blocked}>{t('tasks.blockedBy', { count: task.blockedBy.length })}</span>
        ) : null}
      </div>
      {actions.length > 0 ? (
        <span className={css.rowActions}>
          {actions.includes('claim') ? (
            <Button variant="toolbar" disabled={pending} onClick={() => { onAct(task, 'claim') }}>{t('task.claim')}</Button>
          ) : null}
          {actions.includes('submit') ? (
            <Button variant="toolbar" disabled={pending} onClick={() => { onAct(task, 'submit') }}>{t('task.submit')}</Button>
          ) : null}
          {actions.includes('approve') ? (
            <Button variant="toolbar" disabled={pending} onClick={() => { onReview(true) }}>{t('task.review.approve')}</Button>
          ) : null}
          {actions.includes('reject') ? (
            <Button variant="toolbar" disabled={pending} onClick={() => { onReview(false) }}>{t('task.review.reject')}</Button>
          ) : null}
        </span>
      ) : null}
    </li>
  )
}

/** 任务中心主面板：共享任务/定时计划/后台任务三 tab（web.v1 tasks/schedule 域）。 */
import { useEffect, useState } from 'react'
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-yunyou-remotes/remote'
import type {} from '@deepseek-ai/dsh-client-ui-layout/client'
import type { TaskEntry, TaskCenterSource } from './contract.ts'
import type { TaskCenterStoreHandle } from './stores.ts'
import { ScheduleList } from './ScheduleList.tsx'
import { ScheduleCreateForm } from './ScheduleCreateForm.tsx'
import css from './TaskCenterPage.module.css'

/** 宿主注入面：远程源。 */
export interface TaskCenterInjected {
  readonly source: TaskCenterSource
}

type Tab = 'shared' | 'schedule' | 'jobs'

/** 任务中心页 props（五份额派生）。 */
export type TaskCenterPageProps =
  PropsRuntime<'main'>
  & InjectFace<TaskCenterInjected>
  & PropsStore<TaskCenterStoreHandle>
  & PropsLocale<'taskCenter'>

/**
 * 渲染任务中心：tab 切换共享任务/定时计划/后台任务指引。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 任务中心页。
 */
export function TaskCenterPage(props: TaskCenterPageProps) {
  const { useStore, actions, source, t } = props
  const notify = notifyOf(actions)
  const [tab, setTab] = useState<Tab>('shared')
  const [createOpen, setCreateOpen] = useState(false)
  const tasks = useStore(state => state.tasks)
  const tasksPhase = useStore(state => state.tasksPhase)
  const schedule = useStore(state => state.schedule)
  const schedulePhase = useStore(state => state.schedulePhase)
  const refresh = (): void => {
    actions.setTasks('loading', [])
    actions.setSchedule('loading', [])
    void source.tasks().then(result => {
      actions.setTasks(result.ok ? 'ready' : 'error', result.ok ? result.value.items : [])
    })
    void source.scheduleList().then(result => {
      actions.setSchedule(result.ok ? 'ready' : 'error', result.ok ? result.value.entries : [])
    })
  }
  useEffect(() => { refresh() }, [refresh])
  return (
    <div className={css.page}>
      <header className={css.header}>
        <h1 className={css.title}>{t('page.title')}</h1>
        <div className={css.tabs} role="tablist">
          {(['shared', 'schedule', 'jobs'] as const).map(name => (
            <button
              key={name}
              type="button"
              role="tab"
              aria-selected={tab === name}
              className={css.tab}
              onClick={() => { setTab(name) }}
            >{t(`tab.${name}`)}</button>
          ))}
        </div>
        <Button variant="ghost" onClick={refresh}>{t('page.refresh')}</Button>
      </header>
      {tab === 'shared' ? (
        <section className={css.panel} aria-label={t('tab.shared')}>
          {tasksPhase === 'ready' && tasks.length === 0 ? <p className={css.empty}>{t('tasks.empty')}</p>
            : tasksPhase === 'ready' ? (
              <ul className={css.rows}>
                {tasks.map(task => (
                  <li key={task.id} className={css.row}>
                    <span className={css.badge}>{t(taskStateKey(task.state))}</span>
                    <span className={css.title}>{task.title}</span>
                    <span className={css.meta}>{task.owner === undefined ? '' : `@${task.owner}`}</span>
                  </li>
                ))}
              </ul>
            ) : <p className={css.empty}>{t('page.unavailable')}</p>}
        </section>
      ) : null}
      {tab === 'schedule' ? (
        <section className={css.panel} aria-label={t('tab.schedule')}>
          <div className={css.panelHeader}>
            <Button variant="ghost" onClick={() => { setCreateOpen(true) }}>{t('schedule.create')}</Button>
          </div>
          <ScheduleList t={t} phase={schedulePhase} entries={schedule} source={source} notify={notify} />
          <ScheduleCreateForm
            t={t}
            open={createOpen}
            onClose={() => { setCreateOpen(false) }}
            source={source}
            onCreated={refresh}
          />
        </section>
      ) : null}
      {tab === 'jobs' ? <p className={css.empty}>{t('jobs.empty')}</p> : null}
    </div>
  )
}

/** 状态键显式映射（locale 键类型锁）。 */
function taskStateKey(state: TaskEntry['state']): 'state.pending' | 'state.claimed' | 'state.submitted' | 'state.reviewing' | 'state.approved' | 'state.rejected' {
  return `state.${state as TaskEntry['state']}` as const
}

/** store 动作折叠为通知回调。 */
function notifyOf(actions: PropsStore<TaskCenterStoreHandle>['actions']) {
  return (text: string, level: 'info' | 'error'): void => { actions.setNotice({ text, level }) }
}

export type { Tab }

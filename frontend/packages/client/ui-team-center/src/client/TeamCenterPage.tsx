/** 团队中心主面板：名册与共享任务板分栏（web.v1 team/tasks 域产品面）。 */
import { useEffect, useState } from 'react'
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-layout/client'
import type { TeamStoreHandle } from './stores.ts'
import type { TeamSource } from './contract.ts'
import { RosterPanel } from './RosterPanel.tsx'
import { SpawnDialog } from './SpawnDialog.tsx'
import { TaskBoard } from './TaskBoard.tsx'
import css from './TeamCenterPage.module.css'

/** 宿主注入面：远程源（web.v1 真实 RPC）。 */
export interface TeamCenterInjected {
  readonly source: TeamSource
}

/** 团队中心主面板 props：五份额派生（运行时+注入+store+locale）。 */
export type TeamCenterPageProps =
  PropsRuntime<'main'>
  & InjectFace<TeamCenterInjected>
  & PropsStore<TeamStoreHandle>
  & PropsLocale<'teamCenter'>

/**
 * 渲染团队中心：左名册右任务板；接口未接时两栏显示诚实不可用态。
 * @param props - 框架快照、注入回调与本地化文案。
 * @returns 团队中心页面。
 */
export function TeamCenterPage(props: TeamCenterPageProps) {
  const { useStore, actions, source, t } = props
  const notify = (text: string, level: 'info' | 'error'): void => { actions.setNotice({ text, level }) }
  const refresh = (): void => {
    actions.setRoster('loading', [])
    actions.setTasks('loading', [])
    void source.roster().then(result => {
      if (result.ok) actions.setRoster('ready', result.value)
      else actions.setRoster('error', [])
    })
    void source.tasks().then(result => {
      if (result.ok) actions.setTasks('ready', result.value)
      else actions.setTasks('error', [])
    })
  }
  const [spawnOpen, setSpawnOpen] = useState(false)
  const roster = useStore(state => state.roster)
  const rosterPhase = useStore(state => state.rosterPhase)
  const tasks = useStore(state => state.tasks)
  const tasksPhase = useStore(state => state.tasksPhase)
  // 首挂载拉一次真实数据；后续靠「刷新」与阶段 2 的事件重拉。
  useEffect(() => { refresh() }, [refresh])
  return (
    <div className={css.page}>
      <header className={css.header}>
        <h1 className={css.title}>{t('page.title')}</h1>
        <Button variant="ghost" onClick={refresh}>{t('page.refresh')}</Button>
      </header>
      <div className={css.columns}>
        <RosterPanel
          t={t}
          phase={rosterPhase}
          roster={roster}
          onSpawn={() => { setSpawnOpen(true) }}
          source={source}
          notify={notify}
        />
        <TaskBoard t={t} phase={tasksPhase} tasks={tasks} source={source} notify={notify} />
      </div>
      <SpawnDialog t={t} open={spawnOpen} onClose={() => { setSpawnOpen(false) }} source={source} notify={notify} />
    </div>
  )
}

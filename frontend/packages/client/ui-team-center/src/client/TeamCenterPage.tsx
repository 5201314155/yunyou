/** 团队中心主面板：名册与共享任务板分栏（web.v1 team/tasks 域产品面）。 */
import { useEffect } from 'react'
import { Button } from '@deepseek-ai/dsh-client-ui-primitives'
import type { InjectFace, PropsLocale, PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type { TeamStoreHandle } from './stores.ts'
import type { TeamSource } from './contract.ts'
import { RosterPanel } from './RosterPanel.tsx'
import { TaskBoard } from './TaskBoard.tsx'
import css from './TeamCenterPage.module.css'

/** 宿主注入面：远程源、刷新动作与 store 句柄由挂载点持有。 */
export interface TeamCenterInjected {
  /** 团队远程源（阶段 1=诚实占位，阶段 2=web.v1 真实 RPC）。 */
  readonly source: TeamSource
  /** 重新拉取名册与任务板（一次用户动作或首挂载）。 */
  readonly refresh: () => void
  /** 一次性提示写入（store action 的封装，见 stores.ts）。 */
  readonly notify: (text: string, level: 'info' | 'error') => void
  /** 打开拉人对话框。 */
  readonly openSpawn: () => void
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
  const { useStore, source, refresh, notify, openSpawn, t } = props
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
          onSpawn={openSpawn}
          source={source}
          notify={notify}
        />
        <TaskBoard t={t} phase={tasksPhase} tasks={tasks} source={source} notify={notify} />
      </div>
    </div>
  )
}

/** 任务中心查看态：任务板与定时计划快照。 */
import { defineStore, type EngineStoreHandle } from '@deepseek-ai/dsh-client-store'
import type { LoadPhase, ScheduleListEntry, TaskEntry } from './contract.ts'

/** 一次性操作回执。 */
export interface TaskCenterNotice {
  readonly text: string
  readonly level: 'info' | 'error'
}

export interface TaskCenterData {
  tasksPhase: LoadPhase
  tasks: readonly TaskEntry[]
  schedulePhase: LoadPhase
  schedule: readonly ScheduleListEntry[]
  notice: TaskCenterNotice | null
}

const actions = {
  setTasks: (state: TaskCenterData, phase: LoadPhase, tasks: readonly TaskEntry[]) => {
    state.tasksPhase = phase
    state.tasks = tasks
  },
  setSchedule: (state: TaskCenterData, phase: LoadPhase, schedule: readonly ScheduleListEntry[]) => {
    state.schedulePhase = phase
    state.schedule = schedule
  },
  setNotice: (state: TaskCenterData, notice: TaskCenterNotice | null) => { state.notice = notice },
}

/**
 * 声明任务中心查看态（main 挂载点持有）。
 * @returns store 句柄。
 */
export function createTaskCenterStore(): EngineStoreHandle<TaskCenterData, typeof actions> {
  return defineStore({ init: (): TaskCenterData => ({ tasksPhase: 'idle', tasks: [], schedulePhase: 'idle', schedule: [], notice: null }), actions })
}

export type TaskCenterStoreHandle = ReturnType<typeof createTaskCenterStore>

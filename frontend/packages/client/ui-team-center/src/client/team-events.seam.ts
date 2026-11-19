/**
 * $events 订阅缝桩验证（可执行；vitest 入仓后加 .test.ts 薄包装调 runSeamChecks）。
 * 三例：可选降级 / tasksChanged 订阅触发与退订 / rosterChanged 同。
 */
import type { TeamSource } from './contract.ts'

/** 构建带桩事件的团队源。 */
export function stubSource(): { source: TeamSource; fireTasks: () => void; fireRoster: () => void } {
  const tasksListeners = new Set<() => void>()
  const rosterListeners = new Set<() => void>()
  return {
    source: {
      roster: () => Promise.resolve({ ok: true, value: [] }),
      tasks: () => Promise.resolve({ ok: true, value: [] }),
      spawn: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      message: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      kick: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      claim: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      submit: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      review: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      reassign: () => Promise.resolve({ ok: false, code: 'gateway/not-implemented', message: '' }),
      events: {
        tasksChanged: listener => { tasksListeners.add(listener); return () => { tasksListeners.delete(listener) } },
        rosterChanged: listener => { rosterListeners.add(listener); return () => { rosterListeners.delete(listener) } },
      },
    },
    fireTasks: () => { for (const listener of [...tasksListeners]) listener() },
    fireRoster: () => { for (const listener of [...rosterListeners]) listener() },
  }
}

/** 执行三例断言；全过返回 'ok'，否则抛出首个失败。 */
export function runSeamChecks(): 'ok' {
  const { source, fireTasks, fireRoster } = stubSource()
  // ① 可选缺省=诚实降级
  const bare: TeamSource = { ...source }
  const stripped = bare as { events?: unknown }
  delete stripped.events
  if (stripped.events !== undefined) throw new Error('events 应可缺省')
  // ② tasksChanged 订阅触发一次，退订后不再触发
  let tasksCalls = 0
  const unsubscribeTasks = source.events?.tasksChanged(() => { tasksCalls += 1 })
  fireTasks()
  if (tasksCalls !== 1) throw new Error('tasksChanged 触发次数 ≠1')
  unsubscribeTasks?.()
  fireTasks()
  if (tasksCalls !== 1) throw new Error('退订后仍触发')
  // ③ rosterChanged 同
  let rosterCalls = 0
  const unsubscribeRoster = source.events?.rosterChanged(() => { rosterCalls += 1 })
  fireRoster()
  if (rosterCalls !== 1) throw new Error('rosterChanged 触发次数 ≠1')
  unsubscribeRoster?.()
  fireRoster()
  if (rosterCalls !== 1) throw new Error('退订后仍触发')
  return 'ok'
}

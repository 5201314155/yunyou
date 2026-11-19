/** 任务中心侧栏入口图标（装饰件；可访问名由侧栏行持有）。 */
import { IconChecklistOutlineRegular } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'

/**
 * 渲染侧栏栏位的任务图标。
 * @param props - 侧栏图标份额。
 * @returns 任务图标。
 */
export function TaskCenterIcon({ size }: PropsRuntime<'sidebar.panellist'>) {
  return <IconChecklistOutlineRegular size={size} />
}

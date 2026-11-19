/** 团队中心侧栏入口图标（装饰件；可访问名由侧栏行持有）。 */
import { IconChecklistOutlineRegular } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'

/**
 * 渲染侧栏栏位的团队图标。
 * @param props - 侧栏图标份额：请求尺寸与选中态。
 * @returns 团队图标。
 */
export function TeamCenterIcon({ size }: PropsRuntime<'sidebar.panellist'>) {
  return <IconChecklistOutlineRegular size={size} />
}

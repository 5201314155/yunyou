/** 插件广场侧栏入口图标（装饰件；可访问名由侧栏行持有）。 */
import { IconInspectOutlineRegular } from '@deepseek-ai/dsh-client-ui-primitives'
import type { PropsRuntime } from '@deepseek-ai/dsh-client-ui-slots'
import type {} from '@deepseek-ai/dsh-client-ui-sidebar/client'

/**
 * 渲染侧栏栏位的插件图标。
 * @param props - 侧栏图标份额。
 * @returns 插件图标。
 */
export function PluginCenterIcon({ size }: PropsRuntime<'sidebar.panellist'>) {
  return <IconInspectOutlineRegular size={size} />
}

import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots';
import type { TaskCenterSource } from './contract.ts';
import type { TaskCenterStoreHandle } from './stores.ts';
/** 宿主注入面：远程源。 */
export interface TaskCenterInjected {
    readonly source: TaskCenterSource;
}
type Tab = 'shared' | 'schedule' | 'jobs';
/** 任务中心页 props（五份额派生）。 */
export type TaskCenterPageProps = PropsRuntime<'main'> & InjectFace<TaskCenterInjected> & PropsStore<TaskCenterStoreHandle> & PropsLocale<'taskCenter'>;
/**
 * 渲染任务中心：tab 切换共享任务/定时计划/后台任务指引。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 任务中心页。
 */
export declare function TaskCenterPage(props: TaskCenterPageProps): import("react").JSX.Element;
export type { Tab };
//# sourceMappingURL=TaskCenterPage.d.ts.map
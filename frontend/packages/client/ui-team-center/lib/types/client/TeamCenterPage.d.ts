import type { InjectFace, PropsLocale, PropsRuntime, PropsStore } from '@deepseek-ai/dsh-client-ui-slots';
import type { TeamStoreHandle } from './stores.ts';
import type { TeamSource } from './contract.ts';
/** 宿主注入面：远程源（web.v1 真实 RPC）。 */
export interface TeamCenterInjected {
    readonly source: TeamSource;
}
/** 团队中心主面板 props：五份额派生（运行时+注入+store+locale）。 */
export type TeamCenterPageProps = PropsRuntime<'main'> & InjectFace<TeamCenterInjected> & PropsStore<TeamStoreHandle> & PropsLocale<'teamCenter'>;
/**
 * 渲染团队中心：左名册右任务板；接口未接时两栏显示诚实不可用态。
 * @param props - 框架快照、注入回调与本地化文案。
 * @returns 团队中心页面。
 */
export declare function TeamCenterPage(props: TeamCenterPageProps): import("react").JSX.Element;
//# sourceMappingURL=TeamCenterPage.d.ts.map
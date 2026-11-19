import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 广场卡片：信任徽标+分类+版本数+兼容灰置（§6.2/§6.4）。 */
import { Button } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './PluginCard.module.css';
/** 信任徽标键映射（§6.4 UI 呈现列）。 */
const TRUST_KEY = {
    official: 'card.official',
    signature: 'card.signed',
    unsigned: 'card.unsigned',
};
/**
 * 渲染广场卡片；版本兼容不符或无 Web 面时灰置并给因（不可点装）。
 * @param props - 条目与本地化。
 * @returns 广场卡片。
 */
export function PluginCard({ t, entry, onSelect }) {
    const enginesBlocked = entry.enginesYunyou !== undefined;
    const platformBlocked = !entry.platformWeb;
    const installable = !enginesBlocked && !platformBlocked;
    return (_jsxs("article", { className: css.card, "data-installable": installable || undefined, children: [_jsxs("div", { className: css.head, children: [_jsx("span", { className: css.name, children: entry.name }), _jsx("span", { className: css.trust, "data-trust": entry.trust, children: t(TRUST_KEY[entry.trust]) })] }), _jsx("p", { className: css.description, children: entry.description }), _jsxs("div", { className: css.meta, children: [_jsx("span", { className: css.versions, children: t('card.versions', { count: entry.versions.length }) }), entry.tags.map(tag => _jsx("span", { className: css.tag, children: tag }, tag))] }), enginesBlocked ? (_jsx("p", { className: css.blocked, children: t('card.incompatible', { runtime: 'web', range: entry.enginesYunyou ?? '' }) })) : platformBlocked ? _jsx("p", { className: css.blocked, children: t('card.platformMissing') }) : null, _jsxs("div", { className: css.actions, children: [_jsx(Button, { variant: "outline", onClick: onSelect, children: t('detail.back') === '' ? '' : entry.id }), _jsx(Button, { variant: "primary", disabled: !installable, onClick: onSelect, children: t('card.install') })] })] }));
}
//# sourceMappingURL=PluginCard.js.map
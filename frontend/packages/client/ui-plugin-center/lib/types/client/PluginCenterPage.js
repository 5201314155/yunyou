import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** 插件广场主面板：卡片列表+分类标签+搜索+详情三步文档序（设计 §六）。 */
import { useEffect, useMemo, useState } from 'react';
import { Button } from '@deepseek-ai/dsh-client-ui-primitives';
import { PluginCard } from "./PluginCard.js";
import { PluginDetail } from "./PluginDetail.js";
import css from './PluginCenterPage.module.css';
/**
 * 渲染插件广场：搜索+分类过滤+卡片网格；选中卡片展开三步详情。
 * @param props - 框架快照、注入回调与本地化。
 * @returns 插件广场页。
 */
export function PluginCenterPage(props) {
    const { useStore, actions, source, t } = props;
    const entries = useStore(state => state.entries);
    const phase = useStore(state => state.phase);
    const [query, setQuery] = useState('');
    const [category, setCategory] = useState('all');
    const [selected, setSelected] = useState(undefined);
    const refresh = () => {
        actions.setPhase('loading', []);
        void source.list().then(result => {
            if (result.ok)
                actions.setPhase('ready', result.value.items);
            else
                actions.setPhase('error', []);
        });
    };
    useEffect(() => { refresh(); }, [refresh]);
    const filtered = useMemo(() => entries.filter(entry => {
        if (category !== 'all' && entry.category !== category)
            return false;
        const needle = query.trim().toLowerCase();
        if (needle === '')
            return true;
        return entry.name.toLowerCase().includes(needle) || entry.description.toLowerCase().includes(needle);
    }), [entries, category, query]);
    const categories = ['all', 'tools', 'ui', 'lifecycle', 'documents', 'uncategorized'];
    return (_jsxs("div", { className: css.page, children: [_jsxs("header", { className: css.header, children: [_jsx("h1", { className: css.title, children: t('page.title') }), _jsx("input", { className: css.search, value: query, onChange: event => { setQuery(event.target.value); }, placeholder: t('page.searchPlaceholder'), "aria-label": t('page.searchPlaceholder') }), _jsx(Button, { variant: "ghost", onClick: refresh, children: t('page.refresh') })] }), _jsx("div", { className: css.categories, role: "tablist", children: categories.map(name => (_jsx("button", { type: "button", role: "tab", "aria-selected": category === name, className: css.category, onClick: () => { setCategory(name); }, children: t(`category.${name}`) }, name))) }), selected !== undefined ? (_jsx(PluginDetail, { t: t, entry: selected, source: source, onBack: () => { setSelected(undefined); } })) : phase === 'ready' && filtered.length === 0 ? (_jsx("p", { className: css.empty, children: t('list.empty') })) : phase === 'ready' ? (_jsx("div", { className: css.grid, children: filtered.map(entry => (_jsx(PluginCard, { t: t, entry: entry, onSelect: () => { setSelected(entry); } }, entry.id))) })) : _jsx("p", { className: css.empty, children: t('page.unavailable') })] }));
}
//# sourceMappingURL=PluginCenterPage.js.map
import { jsx as _jsx, jsxs as _jsxs, Fragment as _Fragment } from "react/jsx-runtime";
/** Folded Conversation Views, plugin discovery while reachable, and creation. */
import { Button, IconCheckOutlineRegular, IconPinOutlineRegular, IconPluginPinwheelOutlineRegular, IconPlusOutlineRegular, MenuItemButton, Tooltip, } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './EntryMenu.module.css';
/**
 * Render unpinned sidebar entries or folded Conversation Views, followed by discovery and Creator actions.
 * @param props - Owner dismissal callback, injected actions, and localized copy.
 * @returns Menu rows for the region's More menu.
 */
export function EntryMenuContent({ entries, onOpen, onPinnedChange, region, onDismiss, usePluginNavigation, browsePlugins, createPlugin, t, renderFactorySlot, }) {
    const moreEntries = region === 'sidebar' ? entries.filter(entry => !entry.pinned) : region === 'conversation' ? entries : [];
    const browsable = usePluginNavigation(present => present);
    return _jsxs(_Fragment, { children: [moreEntries.map(entry => _jsxs("div", { className: region === 'sidebar' ? css.sidebarEntry : undefined, children: [_jsx(MenuItemButton, { current: entry.active === true, icon: region === 'sidebar' ? (_jsx("span", { className: css.entryIcon, "aria-hidden": "true", children: renderFactorySlot('sidebar.panel.icon', { id: entry.id, size: 16, active: entry.active === true }) })) : undefined, onSelect: () => { onDismiss(); onOpen(entry.id); }, children: _jsxs("span", { className: css.entryName, children: [_jsx("span", { className: css.entryTitle, children: entry.label }), entry.active === true && _jsx(IconCheckOutlineRegular, { size: 16, className: css.activeMark })] }) }), region === 'sidebar' && _jsx("span", { className: css.pinAnchor, children: _jsx(Tooltip, { label: t('pinTooltip'), side: "bottom", align: "end", delayMs: 500, portal: true, children: _jsx(Button, { size: "sm", role: "menuitem", className: css.pinButton, "aria-label": t('pin', { name: entry.label }), onClick: () => { onDismiss(); onPinnedChange(entry.id, true); }, children: _jsx(IconPinOutlineRegular, { size: 14 }) }) }) })] }, entry.id)), browsable && _jsx(MenuItemButton, { separatorBefore: moreEntries.length > 0, icon: _jsx(IconPluginPinwheelOutlineRegular, { size: 16 }), onSelect: () => { onDismiss(); browsePlugins(); }, children: t('install') }), _jsx(MenuItemButton, { icon: _jsx(IconPlusOutlineRegular, { size: 16 }), separatorBefore: !browsable && moreEntries.length > 0, onSelect: () => { onDismiss(); createPlugin(); }, children: t('create') })] });
}
//# sourceMappingURL=EntryMenuContent.js.map
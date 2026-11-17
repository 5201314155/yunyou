import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** Package-internal row presentation shared by component, data, and child menu entries. */
import { useId } from 'react';
import clsx from 'clsx';
import { IconCheckOutlineRegular, IconChevronRightOutlineRegular } from "./icons/index.js";
import { ShortcutKeys } from "./ShortcutKeys.js";
import { Tooltip } from "./Tooltip.js";
import css from './Menu.module.css';
/** Render a focus-stable row with optional selection, shortcut, and disabled explanation.
 * @param props - row content, native button events, and optional pointer wrapper events.
 * @returns one menu item; explained disabled rows remain focusable but never activate.
 */
export function MenuRow({ children, icon, shortcut, current, disabled = false, disabledReason, danger, separatorBefore, selected, selection = 'check', reserveIcon, decorativeIcon = true, wrapperProps, ...button }) {
    const reasonId = useId();
    const explained = disabled && disabledReason !== undefined;
    // Tooltip remains mounted when the explanation changes, preserving button focus.
    return _jsxs("div", { ...wrapperProps, className: css.itemWrap, children: [separatorBefore && _jsx("div", { className: css.separator, role: "separator" }), _jsx(Tooltip, { label: disabledReason ?? '', disabled: !explained, side: "bottom", delayMs: 500, maxWidth: 300, portal: true, children: _jsxs("button", { ...button, type: "button", role: "menuitem", className: clsx(css.item, selected && (selection === 'fill' ? css.selectedFill : css.selected), danger && css.danger), disabled: disabled && !explained, "aria-disabled": explained || undefined, "aria-describedby": explained ? reasonId : undefined, "aria-keyshortcuts": shortcut?.aria, "aria-current": current === true ? 'page' : undefined, onClick: explained ? undefined : button.onClick, children: [(icon !== undefined || reserveIcon) && _jsx("span", { "aria-hidden": decorativeIcon || undefined, className: css.itemIcon, children: icon }), _jsx("span", { className: css.itemLabel, children: children }), shortcut !== undefined && _jsx("span", { "aria-hidden": "true", className: css.shortcut, children: _jsx(ShortcutKeys, { keys: shortcut.keys, className: css.shortcutKeys }) }), selected && selection === 'check' && _jsx(IconCheckOutlineRegular, { className: css.check }), button['aria-haspopup'] === 'menu' && _jsx(IconChevronRightOutlineRegular, { "aria-hidden": "true", className: css.check })] }) }), explained && _jsx("span", { id: reasonId, hidden: true, children: disabledReason })] });
}
/** Mark icon columns within each separator-delimited data group.
 * @param entries - entries in one menu area.
 * @param contextual - whether to align grouped context-menu icons.
 * @returns entries that reserve a leading icon column.
 */
export function menuIconColumns(entries, contextual) {
    const aligned = new Set();
    if (!contextual)
        return aligned;
    let group = [];
    const finish = () => {
        if (group.some(entry => !('type' in entry) && entry.icon !== undefined)) {
            for (const entry of group)
                aligned.add(entry);
        }
        group = [];
    };
    for (const entry of entries) {
        if ('type' in entry && entry.type === 'separator')
            finish();
        else
            group.push(entry);
    }
    finish();
    return aligned;
}
/** Render a separator or group heading outside the keyboard walk.
 * @param entry - non-interactive menu entry.
 * @returns the corresponding element.
 */
export function renderMenuDecoration(entry) {
    return entry.type === 'separator' ? _jsx("div", { className: css.separator, role: "separator" }, entry.id)
        : _jsx("div", { className: css.label, role: "presentation", children: entry.text }, entry.id);
}
//# sourceMappingURL=MenuRow.js.map
import { jsx as _jsx, Fragment as _Fragment, jsxs as _jsxs } from "react/jsx-runtime";
import { createPortal } from 'react-dom';
import clsx from 'clsx';
import { menuHoverAllowed } from "./useMenuSubmenu.js";
import { menuIconColumns, MenuRow, renderMenuDecoration } from "./MenuRow.js";
import { IconChevronLeftOutlineRegular } from "./icons/index.js";
import { MenuSurface } from "./MenuSurface.js";
import css from './Menu.module.css';
/** Name the parent card anchor, and hide the child card until its layout is known. */
function anchors(submenu, hidden) {
    return { '--dsh-submenu-parent': submenu.parentAnchor, ...hidden ? { visibility: 'hidden' } : {} };
}
/** Render one child list with a non-selecting back row in single-page placement.
 * @param props - active entry, submenu operations, and shared row presentation options.
 * @returns a body portal that preserves child DOM identity across placement changes.
 */
export function SubmenuPanel({ entry, submenu, compact, dense, contextual, backLabel, onSelect, onClose, afterSelection }) {
    const columns = menuIconColumns(entry.submenu, contextual);
    return createPortal(_jsxs(MenuSurface, { ref: submenu.panelRef, compact: compact, className: clsx(css.submenu, css.scrollable, dense && css.denseList, compact && css.compactList), style: anchors(submenu, submenu.hidden), "data-submenu-layout": submenu.measured ? submenu.page ? 'page' : 'side' : undefined, "data-contextual": contextual ? '' : undefined, role: "menu", onPointerEnter: submenu.cancelClose, onPointerLeave: (event) => { if (menuHoverAllowed(event))
            submenu.leave(); }, onContextMenu: contextual ? (event) => { event.preventDefault(); event.stopPropagation(); onClose(); } : undefined, onClick: (event) => { event.stopPropagation(); afterSelection(); }, children: [submenu.page && _jsxs(_Fragment, { children: [_jsx(MenuRow, { "data-menu-back": "", icon: _jsx(IconChevronLeftOutlineRegular, { className: css.check }), onClick: submenu.returnToParent, children: backLabel }), _jsx("div", { className: css.separator, role: "separator" })] }), _jsx("div", { className: css.viewport, role: "presentation", children: entry.submenu.map(sub => 'type' in sub ? renderMenuDecoration(sub)
                    : _jsx(MenuRow, { icon: sub.icon, reserveIcon: columns.has(sub), disabled: sub.disabled, danger: sub.danger, shortcut: sub.shortcut, onClick: () => { onSelect?.(sub.id); }, children: sub.label }, sub.id)) })] }), document.body);
}
//# sourceMappingURL=SubmenuPanel.js.map
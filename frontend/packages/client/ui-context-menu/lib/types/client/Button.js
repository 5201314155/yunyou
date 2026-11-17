import { jsx as _jsx } from "react/jsx-runtime";
/** Shared ellipsis control for explicit context-menu targets. */
import { Button, IconEllipsisOutlineRegular, Tooltip } from '@deepseek-ai/dsh-client-ui-primitives';
import { useId } from 'react';
/**
 * Render the menu trigger while its target kind has a registered contribution.
 * @param props - Localized owner data and framework-bound menu facts.
 * @returns An available target's menu trigger.
 */
export function ContextMenuButton({ target, label, className, size, useOpener, useRegisteredKinds, toggle }) {
    const id = useId();
    const expanded = useOpener(opener => opener === id);
    const available = useRegisteredKinds(kinds => kinds.includes(target.kind));
    if (!available)
        return null;
    const attributes = {
        className,
        'data-context-menu-trigger': '',
        'aria-label': label,
        'aria-haspopup': 'menu',
        'aria-expanded': expanded,
        onPointerDown: (event) => { event.stopPropagation(); },
        onClick: (event) => {
            event.stopPropagation();
            event.currentTarget.focus({ preventScroll: true });
            const rect = event.currentTarget.getBoundingClientRect();
            toggle(id, target, { x: rect.left, y: rect.bottom });
        },
    };
    if (size !== 'sm')
        return _jsx("button", { ...attributes, type: "button", children: _jsx(IconEllipsisOutlineRegular, {}) });
    return _jsx(Tooltip, { label: label, side: "bottom", align: "end", portal: true, children: _jsx(Button, { ...attributes, size: "sm", icon: _jsx(IconEllipsisOutlineRegular, {}) }) });
}
//# sourceMappingURL=Button.js.map
import { jsx as _jsx } from "react/jsx-runtime";
/** Anchored, outside-dismissed popover shell the timing pickers share. */
import { useRef } from 'react';
import { createPortal } from 'react-dom';
import clsx from 'clsx';
import { useDismissOnOutsidePointer } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './PickerPopover.module.css';
/**
 * Render one picker panel into `document.body`, fixed below its trigger.
 *
 * The panel is a portal, so an ancestor's `overflow` cannot crop it, and a
 * pointerdown outside both the trigger and the panel dismisses it.
 * @param props - open state, the trigger, the panel name and layout class, the dismissal callback, and the contents.
 * @returns the panel while open, and nothing while closed.
 */
export function PickerPopover({ open, anchorRef, anchorName, label, className, onClose, children }) {
    const panelRef = useRef(null);
    useDismissOnOutsidePointer(anchorRef, open, onClose, panelRef);
    if (!open)
        return null;
    return createPortal(_jsx("div", { ref: panelRef, className: clsx(css.panel, className), style: { positionAnchor: anchorName }, 
        // The rows that open a picker declare `aria-haspopup="dialog"`, so the
        // panel is the dialog they announce, named by the row's own label.
        role: "dialog", "aria-label": label, 
        // A click inside the panel belongs to the picker, not to the row that
        // opened it: React bubbles portal events through the React tree.
        onClick: (event) => { event.stopPropagation(); }, children: children }), document.body);
}
//# sourceMappingURL=PickerPopover.js.map
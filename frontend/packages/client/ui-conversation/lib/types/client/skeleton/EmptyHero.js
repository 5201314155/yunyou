import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
import { IconChevronDownOutlineRegular, IconFolderCloseRegular, IconFolderOpenRegular, } from '@deepseek-ai/dsh-client-ui-primitives';
import { workspaceTitleOf } from '@deepseek-ai/dsh-util-workspace-path';
import css from './HeroShell.module.css';
/**
 * Basename label for the workspace chip (the shared derivation);
 * separator-only paths echo the raw cwd.
 * @param cwd - workspace directory path (non-empty).
 * @returns chip label.
 */
export function workspaceLabel(cwd) {
    const base = workspaceTitleOf(cwd);
    return base !== '' ? base : cwd;
}
/**
 * The workspace chip (folder + label + chevron), always interactive: before
 * the first message the workspace stays switchable — picking another one
 * moves the New Session flow to that workspace's blank session. Without a
 * label the chip renders its placeholder state: closed folder + the
 * "Choose workspace" call to action.
 * @param props.label - chip label (see {@link workspaceLabel}); omitted → placeholder.
 * @param props.menuOpen - menu expansion echo.
 * @param props.onClick - menu toggle.
 * @returns the chip button element.
 */
export function WorkspaceChip({ buttonRef, label, menuOpen = false, onClick, t }) {
    return (_jsxs("button", { ref: buttonRef, type: "button", className: css.workspace, "aria-label": t('hero.chooseWorkspace'), "aria-haspopup": "menu", "aria-expanded": menuOpen, onClick: onClick, children: [label === undefined
                ? _jsx(IconFolderCloseRegular, { className: css.folder, size: 16 })
                : _jsx(IconFolderOpenRegular, { className: css.folder, size: 16 }), _jsx("span", { className: css.workspaceLabel, children: label ?? t('hero.chooseWorkspace') }), _jsx(IconChevronDownOutlineRegular, { className: css.chevron, size: 12 })] }));
}
/**
 * The hero brand mark: the cloud glyph (34px hitbox, brand cyan), static by
 * design. Decorative — hidden from the accessibility tree.
 * @returns the cloud mark span.
 */
function HeroCloud() {
    return (_jsx("span", { className: css.fish, style: { color: 'var(--dsw-static-deepseek-450)', fontSize: 30, lineHeight: 1 }, "aria-hidden": "true", children: "\u2601" }));
}
/**
 * Render the hero chrome (headline only; no composer, no workspace row).
 * @param props - see {@link HeroShellProps}.
 * @returns the centered hero element tree.
 */
export function HeroShell({ t, renderSlot, children }) {
    return (_jsxs("div", { className: css.root, children: [_jsxs("div", { className: css.stack, children: [_jsxs("div", { className: css.headline, children: [_jsx("span", { className: css.fishHitbox, children: renderSlot('conversation.hero.brand.mark', { size: 34, className: css.fish }, {
                                    fallback: _jsx(HeroCloud, {}),
                                }) }), _jsx("span", { className: css.titleGroup, children: _jsx("span", { children: t('hero.headline') }) })] }), _jsx("div", { className: css.body })] }), children] }));
}
//# sourceMappingURL=EmptyHero.js.map
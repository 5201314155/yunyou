import { jsx as _jsx } from "react/jsx-runtime";
import { IconWarningOutlineRegular, Toast } from '@deepseek-ai/dsh-client-ui-primitives';
/**
 * Show the settled copy outcome outside the Session Header and menu lifetime.
 * @param props - copy notice hook, dismissal action, and localized copy.
 * @returns the success or failure toast, or null while no notice shows.
 */
export function SessionMarkdownCopyToast({ useSessionMarkdownCopy, dismiss, t, }) {
    const notice = useSessionMarkdownCopy(state => state.notice);
    if (notice === null)
        return null;
    return notice.kind === 'success'
        ? _jsx(Toast, { text: t('toast.copied'), tone: "success", onDone: dismiss }, notice.seq)
        : (_jsx(Toast, { text: t(notice.kind === 'preparationFailed' ? 'toast.preparationFailed' : 'toast.clipboardFailed'), icon: _jsx(IconWarningOutlineRegular, {}), onDone: dismiss }, notice.seq));
}
//# sourceMappingURL=CopyToast.js.map
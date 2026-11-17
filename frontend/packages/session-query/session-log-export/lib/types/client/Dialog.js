import { jsx as _jsx } from "react/jsx-runtime";
import { Button, Modal } from '@deepseek-ai/dsh-client-ui-primitives';
/**
 * Show the earliest open export independently of the displayed conversation.
 * @param props - root runtime, bound controller state, dismissal, and localized copy.
 * @returns one modal, or null when all exports are dismissed.
 */
export function SessionLogDownloadDialog({ useSessionLogDownload, dismiss, t, }) {
    const bySession = useSessionLogDownload(state => state.bySession);
    const pending = Object.entries(bySession)
        .filter((pair) => pair[1]?.open === true)
        .sort((a, b) => a[1].order - b[1].order)[0];
    if (pending === undefined)
        return null;
    const [id, entry] = pending;
    const sessionId = id;
    const { status } = entry;
    const title = status === 'downloading'
        ? t('dialog.preparingTitle')
        : status === 'success' ? t('dialog.successTitle') : t('dialog.errorTitle');
    const description = status === 'downloading'
        ? t('dialog.preparingDescription')
        : status === 'success' ? t('dialog.successDescription') : entry.error || t('dialog.commandFailed');
    return (_jsx(Modal, { open: true, onClose: () => { dismiss(sessionId); }, title: title, description: `${t('dialog.session', { sessionId })} ${description}`, closeLabel: t('dialog.close'), footer: _jsx(Button, { variant: "primary", onClick: () => { dismiss(sessionId); }, children: t('dialog.close') }) }));
}
//# sourceMappingURL=Dialog.js.map
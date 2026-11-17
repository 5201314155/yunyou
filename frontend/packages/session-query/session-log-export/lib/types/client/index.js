/** Browser plugin owning ZIP download and Markdown copy state, plus feedback controls. */
import { createSnapshotStore } from '@deepseek-ai/dsh-client-store';
import { IconCopyOutlineRegular, IconDownloadOutlineRegular, IconPaperPlaneOutlineRegular } from '@deepseek-ai/dsh-client-ui-primitives';
import { SessionLogDownloadController } from "./controller.js";
import { SessionMarkdownCopyController } from "./copy-controller.js";
import { SessionMarkdownCopyToast } from "./CopyToast.js";
import { SessionLogDownloadDialog } from "./Dialog.js";
import { en, ja, es, fr, de, ru, pt, NS, zh } from "./locales.js";
export const inject = ['slots', 'locale', 'contextMenus'];
/**
 * Provide the download controller and mount its modal, the Markdown copy
 * controller and its root toast, and the shared Session menu actions.
 * @param ctx - browser context carrying slots and locale services.
 */
export function apply(ctx) {
    const controller = new SessionLogDownloadController();
    const copyController = new SessionMarkdownCopyController();
    ctx.provide('sessionLogDownload', controller);
    ctx.effect(() => async () => { await controller.dispose(); }, 'session-log-download: browser download lifecycle');
    ctx.effect(() => async () => { await copyController.dispose(); }, 'session-log-download: browser Markdown copy lifecycle');
    ctx.effect(() => ctx.locale.register(NS, { zh, en, ja, es, fr, de, ru, pt }), 'session-log-download: browser dictionaries');
    const feedbackAvailable = createSnapshotStore(false);
    ctx.inject(['feedbackUi'], (scope) => {
        scope.effect(() => {
            feedbackAvailable.set(true);
            return () => { feedbackAvailable.set(false); };
        }, 'session-log-download: feedback availability');
    });
    const t = ctx.locale.bind(NS);
    ctx.slots.inject('contextMenu.icon', function* () {
        for (const [key, Icon] of Object.entries({
            'session-log-download.copy': IconCopyOutlineRegular,
            'session-log-download.download': IconDownloadOutlineRegular,
            'session-log-download.feedback': IconPaperPlaneOutlineRegular,
        }))
            yield ctx.slots.register({ name: 'contextMenu.icon', key }, Icon);
    });
    ctx.effect(() => ctx.contextMenus.register({
        id: 'session-log-download.export', target: 'sessionHeader', order: 100,
        observe: () => [controller.store, copyController.store, ctx.locale],
        resolve: ({ sessionId }) => [
            { id: 'copy', label: t('menu.copyMarkdown'), icon: { key: 'session-log-download.copy' },
                disabled: copyController.store.getSnapshot().copying, run: () => copyController.copy(sessionId) },
            { id: 'download', label: t('menu.download'), icon: { key: 'session-log-download.download' },
                disabled: controller.store.getSnapshot().bySession[sessionId]?.status === 'downloading',
                run: () => controller.download(sessionId) },
        ],
    }), 'session-log-download: Header exports');
    ctx.effect(() => ctx.contextMenus.register({
        id: 'session-log-download.feedback', target: 'sessionHeader', order: 600,
        observe: () => [feedbackAvailable, ctx.locale],
        resolve: ({ sessionId }) => feedbackAvailable.getSnapshot() ? [{
                id: 'feedback', label: t('menu.feedback'), icon: { key: 'session-log-download.feedback' }, separatorBefore: true,
                // The feedback plugin can unload between menu preparation and selection.
                run: () => { ctx.get('feedbackUi')?.openSession(sessionId); },
            }] : [],
    }), 'session-log-download: Header feedback');
    ctx.on('command/executed', (sessionId, commandName, result) => {
        if (commandName === 'export' && result.kind === 'success')
            void controller.download(sessionId);
    });
    ctx.slots.inject('shell.overlay', () => ctx.slots.register({
        name: 'shell.overlay',
        id: 'session-log-download.copy-toast',
        locale: NS,
        inject: () => ({
            hooks: { sessionMarkdownCopy: copyController.store },
            dismiss: () => { copyController.dismiss(); },
        }),
    }, SessionMarkdownCopyToast));
    ctx.slots.inject('shell.overlay', () => ctx.slots.register({
        name: 'shell.overlay',
        id: 'session-log-download',
        locale: NS,
        inject: () => ({
            hooks: { sessionLogDownload: controller.store },
            dismiss: (sessionId) => { controller.dismiss(sessionId); },
        }),
    }, SessionLogDownloadDialog));
}
//# sourceMappingURL=index.js.map
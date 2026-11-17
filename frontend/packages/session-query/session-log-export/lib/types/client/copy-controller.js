/**
 * Browser controller for the Session Header's "Copy Session as Markdown" action. One
 * plugin instance owns one pending copy: the clipboard is a browser-wide
 * destination, so a second request joins the pending operation instead of
 * racing it.
 *
 * On hosts with `ClipboardItem` and `navigator.clipboard.write`, the controller
 * starts that write synchronously from the click with a promised text Blob, so
 * the browser keeps the user activation while the Host prepares the
 * transcript. Other hosts fall back to `writeClipboard` after preparation. A
 * disposal aborts the read and prevents later text from reaching the Blob, but
 * an accepted operating-system clipboard write cannot be rolled back.
 * @module
 */
import { createSnapshotStore } from '@deepseek-ai/dsh-client-store';
import { writeClipboard } from '@deepseek-ai/dsh-client-ui-primitives';
import { SESSION_MARKDOWN_EXPORT_ROUTE } from "../routes.js";
const INITIAL = { copying: false, notice: null };
const browserClipboard = {
    supportsDeferredWrite() {
        const clipboard = Reflect.get(navigator, 'clipboard');
        return typeof globalThis.ClipboardItem === 'function'
            && typeof clipboard === 'object' && clipboard !== null
            && typeof Reflect.get(clipboard, 'write') === 'function';
    },
    writeDeferred(data) {
        return navigator.clipboard.write([new ClipboardItem({ 'text/plain': data })]);
    },
    writeText: writeClipboard,
};
function startWrite(start) {
    try {
        return start();
    }
    catch (error) {
        return Promise.reject(new Error('clipboard write could not start', { cause: error }));
    }
}
/** Owns the one pending Markdown copy for this plugin instance. */
export class SessionMarkdownCopyController {
    fetcher;
    clipboard;
    /** uSES-safe state source shared by every Header row and the overlay toast. */
    store = createSnapshotStore(INITIAL);
    pending;
    disposed = false;
    noticeSeq = 0;
    constructor(fetcher = (input, init) => fetch(input, init), clipboard = browserClipboard) {
        this.fetcher = fetcher;
        this.clipboard = clipboard;
    }
    /**
     * Copy one Session's complete transcript. A request received while a copy is
     * pending joins that operation and keeps its originally captured Session.
     * @param sessionId - Session captured by the click.
     * @returns after the clipboard work settles, a failure notice is published, or a late post-disposal request is ignored.
     */
    copy(sessionId) {
        if (this.disposed)
            return Promise.resolve();
        if (this.pending !== undefined)
            return this.pending.done;
        const abort = new AbortController();
        const done = this.run(sessionId, abort.signal).finally(() => {
            this.pending = undefined;
        });
        this.pending = { abort, done };
        return done;
    }
    /** Clear the shown toast without affecting a pending copy. */
    dismiss() {
        if (this.store.getSnapshot().notice === null)
            return;
        this.store.update((state) => { state.notice = null; });
    }
    /**
     * Abort a pending read, suppress a late clipboard write or notice, and reach
     * quiescence. A write already accepted by the operating system cannot be
     * undone.
     * @returns after the pending operation and any admitted clipboard work settle.
     */
    async dispose() {
        this.disposed = true;
        const pending = this.pending;
        pending?.abort.abort();
        await pending?.done;
        this.store.set(INITIAL);
    }
    run(sessionId, signal) {
        this.store.update((state) => { state.copying = true; });
        const preparation = this.prepare(sessionId, signal);
        if (this.clipboard.supportsDeferredWrite()) {
            const data = preparation.then((markdown) => {
                if (this.isCancelled() || signal.aborted)
                    throw new Error('Session Markdown copy cancelled');
                return new Blob([markdown], { type: 'text/plain' });
            });
            const write = startWrite(() => this.clipboard.writeDeferred(data));
            return this.settleDeferred(preparation, data, write);
        }
        return this.settleFallback(preparation);
    }
    async prepare(sessionId, signal) {
        const query = new URLSearchParams({ sessionId });
        const route = `${SESSION_MARKDOWN_EXPORT_ROUTE}?${query.toString()}`;
        const response = await this.fetcher(route, { signal });
        if (!response.ok)
            throw new Error(`Session Markdown request failed: HTTP ${String(response.status)}`);
        return response.text();
    }
    async settleDeferred(preparation, data, write) {
        try {
            const [prepared, blob, written] = await Promise.allSettled([preparation, data, write]);
            if (this.isCancelled())
                return;
            if (prepared.status === 'rejected' || blob.status === 'rejected') {
                this.publishNotice('preparationFailed');
                return;
            }
            if (written.status === 'rejected') {
                this.publishNotice('clipboardFailed');
                return;
            }
            this.publishNotice('success');
        }
        finally {
            this.store.update((state) => { state.copying = false; });
        }
    }
    async settleFallback(preparation) {
        try {
            let markdown;
            try {
                markdown = await preparation;
            }
            catch {
                if (!this.isCancelled())
                    this.publishNotice('preparationFailed');
                return;
            }
            if (this.isCancelled())
                return;
            let accepted = false;
            try {
                accepted = await this.clipboard.writeText(markdown);
            }
            catch {
                accepted = false;
            }
            if (this.isCancelled())
                return;
            this.publishNotice(accepted ? 'success' : 'clipboardFailed');
        }
        finally {
            this.store.update((state) => { state.copying = false; });
        }
    }
    isCancelled() {
        return this.disposed;
    }
    publishNotice(kind) {
        this.store.update((state) => {
            state.notice = { kind, seq: ++this.noticeSeq };
        });
    }
}
//# sourceMappingURL=copy-controller.js.map
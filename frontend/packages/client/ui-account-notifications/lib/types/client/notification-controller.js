/**
 * @param displayUntil - server expiration timestamp.
 * @param at - milliseconds since epoch.
 * @returns whether the notification can no longer be shown.
 */
function expired(displayUntil, at) {
    if (displayUntil === null)
        return false;
    const expires = Date.parse(displayUntil);
    // An unparsable timestamp is not evidence of expiry, so the notice still shows.
    return !Number.isNaN(expires) && expires <= at;
}
/**
 * Own the read, display, and interaction reporting lifecycle of account notifications.
 * @param options - collaborators and timings.
 * @returns the controller the account plugin drives from account and settings state.
 */
export function createNotificationController(options) {
    let scope;
    let locale = options.locale;
    let contentGeneration = 0;
    /** Notice currently published to the sidebar, including the expiration checked at display time. */
    let current = [];
    let poll;
    /** Notifications the card on screen already reported, so one mount cannot report display twice. */
    const reported = new Set();
    const setCurrent = (notices) => {
        if (notices.length === current.length && notices.every((notice, index) => {
            return JSON.stringify(notice) === JSON.stringify(current[index]);
        }))
            return;
        const retained = new Set(notices.map(notice => notice.id));
        for (const id of reported)
            if (!retained.has(id))
                reported.delete(id);
        current = notices;
        options.publish(notices);
    };
    /**
     * Keep a presented foreground card until dismissal or expiration, refreshing
     * its copy when offered again; other live candidates follow in server order.
     */
    const apply = (batch, preserveCopy) => {
        const notifications = batch.notifications.map((notice) => {
            const previous = preserveCopy ? current.find(value => value.id === notice.id) : undefined;
            return previous === undefined ? notice : { ...notice, title: previous.title, body: previous.body, actions: previous.actions };
        });
        const foreground = current[0];
        const retained = foreground !== undefined && reported.has(foreground.id)
            ? notifications.find(notice => notice.id === foreground.id) ?? foreground
            : undefined;
        const at = Date.now();
        const candidates = notifications.filter(notice => !expired(notice.displayUntil, at));
        setCurrent(retained !== undefined && !expired(retained.displayUntil, at)
            ? [retained, ...candidates.filter(notice => notice.id !== retained.id)]
            : candidates);
    };
    /** Refresh only cards that still belong to this account and language when the replies settle. */
    async function refreshContent() {
        const scopeAtCall = scope;
        if (scopeAtCall === undefined || current.length === 0)
            return;
        const generation = ++contentGeneration;
        const requestedLocale = locale;
        const responses = await Promise.allSettled(current.map(async (notice) => options.readContent(notice.id, requestedLocale)));
        if (scope !== scopeAtCall || generation !== contentGeneration)
            return;
        if (responses.some(response => response.status === 'rejected'))
            console.error('[deepseek-account] notification content read failed');
        const updates = responses.flatMap(response => response.status === 'fulfilled' && response.value !== null ? [response.value] : []);
        setCurrent(current.map((notice) => {
            const update = updates.find(value => value.id === notice.id);
            return update == null ? notice : { ...notice, title: update.title, body: update.body, actions: update.actions };
        }));
    }
    /**
     * Read once for the active poll, retaining cards on failure. Replacement and
     * sign-out discard late replies and prevent another read from being scheduled.
     * @param pollAtRead - query cycle that owns this read.
     * @returns after this read settles.
     */
    async function runRead(pollAtRead) {
        const requestedLocale = locale;
        let batch;
        try {
            batch = await options.read(requestedLocale);
        }
        catch (_error) {
            if (poll !== pollAtRead)
                return;
            console.error('[deepseek-account] notification read failed; retaining cards and retrying on the next poll');
            batch = null;
        }
        if (poll !== pollAtRead)
            return;
        if (batch !== null) {
            pollAtRead.intervalMs = batch.nextPollAfterSeconds * 1_000;
            apply(batch, requestedLocale !== locale);
            if (requestedLocale !== locale)
                await refreshContent();
        }
        if (poll !== pollAtRead)
            return;
        pollAtRead.timer = setTimeout(() => { void runRead(pollAtRead); }, pollAtRead.intervalMs);
    }
    const stopPolling = () => {
        if (poll?.timer !== undefined)
            clearTimeout(poll.timer);
        poll = undefined;
    };
    const restartPolling = () => {
        stopPolling();
        poll = { intervalMs: options.pollIntervalMs };
        return runRead(poll);
    };
    /** Attempt each interaction once without changing the card on failure. */
    const report = async (id, event) => {
        const scopeAtReport = scope;
        if (scopeAtReport === undefined || current[0]?.id !== id || (event === 'displayed' && reported.has(id)))
            return;
        if (event === 'displayed')
            reported.add(id);
        try {
            await options.reportEvent(id, event);
        }
        catch (_error) {
            if (scope === scopeAtReport) {
                console.error('[deepseek-account] notification event report failed; keeping local state without retry', { event });
            }
        }
    };
    const endLifecycle = () => {
        if (scope === undefined)
            return;
        stopPolling();
        scope = undefined;
        contentGeneration++;
        reported.clear();
        setCurrent([]);
    };
    return {
        begin() {
            scope ??= {};
            void restartPolling();
        },
        end: endLifecycle,
        refresh() {
            if (scope === undefined)
                return Promise.resolve();
            return restartPolling();
        },
        async setLocale(value) {
            if (value === locale)
                return;
            locale = value;
            contentGeneration++;
            await refreshContent();
        },
        shown(id) {
            setCurrent(current.filter(notice => !expired(notice.displayUntil, Date.now())));
            if (current[0]?.id !== id)
                return;
            void report(id, 'displayed');
        },
        ctaClicked(id) {
            setCurrent(current.filter(notice => !expired(notice.displayUntil, Date.now())));
            if (current[0]?.id !== id)
                return;
            void report(id, 'cta_clicked');
        },
        dismiss(id) {
            setCurrent(current.filter(notice => !expired(notice.displayUntil, Date.now())));
            if (current[0]?.id !== id)
                return;
            void report(id, 'dismissed');
            setCurrent(current.slice(1).filter(notice => !expired(notice.displayUntil, Date.now())));
        },
    };
}
//# sourceMappingURL=notification-controller.js.map
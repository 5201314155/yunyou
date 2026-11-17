import { jsx as _jsx, jsxs as _jsxs } from "react/jsx-runtime";
/** Notification extension rendered inside the sidebar account menu. */
import { useEffect, useRef } from 'react';
import { Button, IconCloseOutlineRegular } from '@deepseek-ai/dsh-client-ui-primitives';
import css from './AccountNotifications.module.css';
import { dismissNotice } from "./notice-transition.js";
/** @param props - account-menu availability and notification content. @returns the anchored notification stack. */
export function AccountNotifications({ active, openAccount, useNotices, shown, ctaClicked, dismiss, t }) {
    const notices = useNotices(value => value);
    return active && _jsx(AccountNoticeStack, { notices: notices, closeLabel: t('close'), onOpenAccount: openAccount, onShown: shown, onCtaClicked: ctaClicked, onDismiss: dismiss });
}
/* v8 ignore next 3 -- closed-union defaults only defend future source widening */
function assertNever(_value) {
    throw new Error('unsupported account notification target');
}
/** @param displayUntil - server display deadline. @returns whether the notification remains eligible for display. */
function live(displayUntil) {
    if (displayUntil === null)
        return true;
    const expires = Date.parse(displayUntil);
    // An unparsable timestamp is not evidence of expiry, so the notice still shows.
    return Number.isNaN(expires) || expires > Date.now();
}
/**
 * @param run - callback to invoke once one presented frame has passed.
 * @returns a canceller for a card that unmounts or hides first.
 */
function afterPaint(run) {
    if (typeof requestAnimationFrame !== 'function') {
        // Without a frame clock, the following task is the best available paint boundary.
        const timer = setTimeout(run, 0);
        return () => { clearTimeout(timer); };
    }
    let cancelled = false;
    let timer;
    const frame = requestAnimationFrame(() => {
        // The frame callback runs before its paint; the following task runs after it.
        timer = setTimeout(() => {
            if (!cancelled)
                run();
        }, 0);
    });
    return () => {
        cancelled = true;
        cancelAnimationFrame(frame);
        if (timer !== undefined)
            clearTimeout(timer);
    };
}
/** @param props - server-ordered notices, localized fallback copy, and display callbacks.
 * @returns a non-modal stack whose foreground card alone can report display or be closed.
 */
export function AccountNoticeStack({ notices, closeLabel, onShown, onCtaClicked, onDismiss, onOpenAccount }) {
    const liveNotices = notices.filter(notice => live(notice.displayUntil));
    return _jsx("div", { className: css.stack, children: liveNotices.map((notice, depth) => _jsx(NoticeCard, { notice: notice, depth: depth, closeLabel: closeLabel, onShown: onShown, onCtaClicked: onCtaClicked, onDismiss: onDismiss, onOpenAccount: onOpenAccount }, notice.id)) });
}
/** One stable card node remains mounted as the cards before it are dismissed. */
function NoticeCard({ notice, depth, closeLabel, onShown, onCtaClicked, onDismiss, onOpenAccount }) {
    const foreground = depth === 0;
    const card = useRef(null);
    const reported = useRef(false);
    const actionSlots = new Map();
    useEffect(() => {
        if (!foreground || reported.current || !live(notice.displayUntil))
            return;
        const element = card.current;
        /* v8 ignore next -- React attaches the card ref before running this effect. */
        if (element === null)
            return;
        let intersecting = false;
        let cancel;
        const visible = () => intersecting && document.visibilityState !== 'hidden' && !element.closest('[hidden], [inert]');
        const attempt = () => {
            if (reported.current || cancel !== undefined || !live(notice.displayUntil))
                return;
            if (!visible())
                return;
            cancel = afterPaint(() => {
                cancel = undefined;
                if (reported.current || !visible() || !live(notice.displayUntil))
                    return;
                reported.current = true;
                onShown(notice.id);
            });
        };
        const observer = new IntersectionObserver((entries) => {
            for (const entry of entries) {
                intersecting = entry.isIntersecting;
                attempt();
            }
        });
        observer.observe(element);
        document.addEventListener('visibilitychange', attempt);
        return () => {
            observer.disconnect();
            document.removeEventListener('visibilitychange', attempt);
            cancel?.();
        };
    }, [foreground, notice.id, notice.displayUntil, onShown]);
    return _jsxs("aside", { ref: card, className: css.card, "data-front": foreground, "aria-hidden": !foreground, style: { '--notice-depth': depth, '--notice-scale': Math.pow(252 / 260, depth) }, children: [_jsxs("div", { role: "status", className: css.copy, children: [_jsxs("div", { className: css.title, children: [_jsx("span", { className: css.icon, "aria-hidden": "true", children: _jsxs("svg", { width: "14", height: "14", viewBox: "0 0 14 14", fill: "none", children: [_jsx("path", { d: "M3.6355 6.93969C3.6355 4.8398 5.33779 3.1375 7.43768 3.1375C9.53757 3.1375 11.23987 4.8398 11.23987 6.93968V11.363H3.6355V6.93969Z", fill: "currentColor", fillOpacity: "0.2", stroke: "currentColor", strokeWidth: "0.875" }), _jsx("path", { d: "M1.641 11.3645H12.36M5.053 13.3095H8.9479M7.4375 4.27422V2.70137912", stroke: "currentColor", strokeWidth: "0.875" })] }) }), _jsx("span", { children: notice.title })] }), _jsx("p", { children: notice.body })] }), notice.actions.length > 0 && _jsx("div", { className: css.actions, children: notice.actions.map((action) => {
                    const target = JSON.stringify(action.target);
                    const occurrence = actionSlots.get(target) ?? 0;
                    actionSlots.set(target, occurrence + 1);
                    return _jsx(Button, { size: "sm", className: css.action, variant: action.style === 'primary' ? 'primary' : 'outline', tabIndex: foreground ? undefined : -1, onClick: (event) => {
                            if (action.target.type !== 'dismiss')
                                onCtaClicked(notice.id);
                            switch (action.target.type) {
                                case 'app_route':
                                    onOpenAccount();
                                    break;
                                case 'dismiss':
                                    dismissNotice(event.currentTarget.closest('aside'), () => { onDismiss(notice.id); });
                                    break;
                                /* v8 ignore next -- the Host validates both supported target variants */
                                default: assertNever(action.target);
                            }
                        }, children: action.label }, `${target}:${occurrence}`);
                }) }), _jsx("button", { type: "button", tabIndex: foreground ? undefined : -1, className: css.close, "aria-label": closeLabel, onClick: (event) => { dismissNotice(event.currentTarget.parentElement, () => { onDismiss(notice.id); }); }, children: _jsx(IconCloseOutlineRegular, { size: 10 }) })] });
}
//# sourceMappingURL=AccountNotifications.js.map
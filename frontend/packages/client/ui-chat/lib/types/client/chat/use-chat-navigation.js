/** Turn jumps and history-prepend anchoring, independent of DOM measurement. */
import { useLayoutEffect, useState } from 'react';
/** Owns one replaceable turn jump and the anchor retained while history loads. */
export class ChatNavigation {
    viewport;
    reading;
    input;
    onBusyTurn;
    onHistoryIntent;
    jump = null;
    settleFrame = null;
    historyFrame = null;
    historyIntent = 'none';
    historyRequested = false;
    constructor(viewport, reading, input, onBusyTurn, onHistoryIntent = () => { }) {
        this.viewport = viewport;
        this.reading = reading;
        this.input = input;
        this.onBusyTurn = onBusyTurn;
        this.onHistoryIntent = onHistoryIntent;
    }
    /**
     * Adopt committed history availability without starting a request.
     * @param input - history state from the latest committed render.
     */
    setInput(input) {
        this.input = input;
        this.historyRequested = false;
        if (!input.isOpen || input.historyLoadStatus === 'error' || !input.hasMore)
            this.setHistoryIntent('none');
    }
    /** Cancel navigation when opening a Chat view. */
    reset() {
        this.cancel();
    }
    /** Cancel local callbacks; late history completions cannot revive a task. */
    dispose() {
        this.clearTask();
        if (this.historyFrame !== null)
            cancelAnimationFrame(this.historyFrame);
        this.historyFrame = null;
    }
    /** Release the jump, paging anchor, and busy indicator without cancelling shared history I/O. */
    cancel() {
        this.setHistoryIntent('none');
        this.clearTask();
        this.onBusyTurn(null);
    }
    /** Release a jump after reader input without cancelling an accepted history click. */
    readerInteracted() {
        this.clearTask();
        this.onBusyTurn(null);
    }
    clearTask() {
        this.cancelFrame();
        this.jump = null;
        this.viewport.stopPreserving();
        this.viewport.cancelHistoryPrepend();
    }
    /**
     * Replace the current jump with an explicit turn selection.
     * @param item - loaded anchor or unloaded turn to fetch before landing.
     */
    navigateToTurn = (item) => {
        if (item.anchor.kind === 'loaded') {
            this.cancel();
            const landing = this.viewport.scrollToTurn(item.turn);
            if (landing === null)
                return;
            this.reading.acceptNavigation(landing);
            if (this.input.loadingOlder)
                this.viewport.beginPreserving(landing.position);
            return;
        }
        this.cancel();
        this.viewport.beginPreserving();
        this.reading.pauseFollowing();
        const jump = {
            turn: item.turn,
            seq: item.anchor.seq,
            phase: 'loading',
            landing: 'pending',
            repageHead: null,
        };
        this.jump = jump;
        this.onBusyTurn(jump.turn);
        this.request(jump);
    };
    /** Request and consume one page, retaining the click until success, exhaustion, or failure. */
    loadEarlier = () => {
        this.cancel();
        this.setHistoryIntent('manual');
        this.reading.pauseFollowing();
        if (this.input.historyLoadStatus === 'error') {
            this.historyRequested = true;
            this.input.loadOlder();
        }
        else
            this.requestHistoryCheck();
    };
    setHistoryIntent(intent) {
        if (intent === this.historyIntent)
            return;
        this.historyIntent = intent;
        this.onHistoryIntent(intent);
    }
    /** Coalesce event-driven checks; waiting never schedules its own next frame. */
    requestHistoryCheck() {
        if (!this.input.isOpen || this.input.historyLoadStatus === 'error')
            return;
        if (this.historyIntent === 'none' && (this.input.historyLoadStatus !== 'idle'
            || !this.input.hasMore || this.reading.followingTail))
            return;
        if (this.historyFrame !== null || typeof requestAnimationFrame !== 'function')
            return;
        this.historyFrame = requestAnimationFrame(() => {
            this.historyFrame = null;
            this.checkHistory();
        });
    }
    checkHistory() {
        const input = this.input;
        if (!input.isOpen || this.jump !== null || this.historyRequested || input.loadingOlder)
            return;
        if (input.historyLoadStatus === 'error')
            return;
        if (input.historyLoadStatus === 'pendingConsumption') {
            if (this.historyIntent === 'none')
                return;
            if (this.reading.followingTail && this.historyIntent === 'auto') {
                this.setHistoryIntent('none');
                return;
            }
            if (this.historyIntent === 'manual') {
                this.viewport.beginManualPaging();
                this.reading.pauseFollowing();
                this.consumeHistory('preserve');
                return;
            }
            const mode = this.viewport.historyConsumption();
            switch (mode) {
                case 'touch':
                case 'inner-scroll':
                    return;
                case 'motion':
                case 'pointer':
                case 'unavailable':
                    this.setHistoryIntent('none');
                    return;
                case 'prepend':
                    if (!this.viewport.beginHistoryPrepend() && input.firstSeq !== null) {
                        this.setHistoryIntent('none');
                        return;
                    }
                    break;
                case 'preserve':
                case 'native':
                    break;
            }
            if (mode === 'preserve') {
                this.viewport.beginPreserving();
                if (!this.viewport.preserving)
                    this.viewport.beginPaging();
                if (!this.viewport.preserving && input.firstSeq !== null) {
                    this.setHistoryIntent('none');
                    return;
                }
                this.reading.pauseFollowing();
            }
            this.consumeHistory(mode);
            return;
        }
        if (!input.hasMore || this.historyIntent !== 'manual' && (this.reading.followingTail
            || !this.viewport.confirmHistoryStart()))
            return;
        if (this.historyIntent === 'none')
            this.setHistoryIntent('auto');
        this.historyRequested = true;
        input.loadOlder();
    }
    consumeHistory(mode) {
        this.historyRequested = true;
        if (this.input.consumeOlder())
            this.setHistoryIntent('none');
        else {
            this.historyRequested = false;
            if (mode === 'preserve')
                this.viewport.stopPreserving();
            if (mode === 'prepend')
                this.viewport.cancelHistoryPrepend();
        }
    }
    /**
     * Preserve reader ownership across pending history work.
     * @param sample - settled reader movement that can update or interrupt an anchor.
     */
    readerSampled(sample) {
        if (sample.movedByReader && this.jump?.landing === 'landed')
            this.jump.landing = 'interrupted';
        if (sample.followingTail || sample.movedByReader)
            this.viewport.stopPreserving();
    }
    /**
     * Preserve one paging anchor after a commit or a later size change, regardless of head identity.
     * @returns whether the retained anchor handled the layout change.
     */
    contentCommitted() {
        if (!this.viewport.preserving || this.reading.pending)
            return false;
        if (this.landJump(false))
            return true;
        const landing = this.viewport.preserve();
        if (landing === null)
            return false;
        this.reading.preservePosition(landing);
        return true;
    }
    /** Land, retry, or complete the current jump against the committed window. */
    reconcile() {
        const jump = this.jump;
        if (jump === null || this.reading.pending)
            return;
        if (jump.phase === 'loading') {
            if (jump.landing === 'pending')
                this.landJump(false);
            return;
        }
        if (this.input.loadingOlder)
            return;
        if (this.landJump(true))
            return;
        const uncovered = this.input.firstSeq === null || this.input.firstSeq > jump.seq;
        if (uncovered && this.input.hasMore && jump.repageHead !== this.input.firstSeq) {
            jump.repageHead = this.input.firstSeq;
            this.viewport.beginPreserving();
            this.request(jump);
            return;
        }
        const fallback = this.viewport.scrollToTurnAtOrAfter(jump.turn);
        this.cancel();
        if (fallback !== null)
            this.reading.acceptNavigation(fallback);
    }
    landJump(settle) {
        const jump = this.jump;
        if (jump === null)
            return false;
        if (jump.landing === 'interrupted') {
            if (settle) {
                this.cancel();
                return true;
            }
            return false;
        }
        const landing = this.viewport.scrollToTurn(jump.turn);
        if (landing === null)
            return false;
        this.reading.acceptNavigation(landing);
        if (settle)
            this.cancel();
        else {
            this.viewport.beginPreserving(landing.position);
            jump.landing = 'landed';
        }
        return true;
    }
    request(jump) {
        jump.phase = 'loading';
        const settled = () => {
            if (this.jump !== jump)
                return;
            jump.phase = 'settled';
            this.cancelFrame();
            if (typeof requestAnimationFrame !== 'function')
                this.reconcile();
            else
                this.settleFrame = requestAnimationFrame(() => {
                    this.settleFrame = null;
                    if (this.jump === jump)
                        this.reconcile();
                });
        };
        void this.input.loadThrough(jump.seq).then(settled, settled);
    }
    cancelFrame() {
        if (this.settleFrame !== null && typeof cancelAnimationFrame === 'function')
            cancelAnimationFrame(this.settleFrame);
        this.settleFrame = null;
    }
}
/**
 * Retain one navigation owner for the component's lifetime.
 * @param viewport - turn-aware DOM operations.
 * @param reading - reading and follow policy receiving navigation landings.
 * @param input - committed history state and load operations.
 * @returns the navigation owner and its visible busy turn.
 */
export function useChatNavigation(viewport, reading, input) {
    const [busyTurn, setBusyTurn] = useState(null);
    const [historyIntent, setHistoryIntent] = useState('none');
    const [navigation] = useState(() => new ChatNavigation(viewport, reading, input, setBusyTurn, setHistoryIntent));
    useLayoutEffect(() => { navigation.setInput(input); }, [navigation, input]);
    useLayoutEffect(() => () => { navigation.dispose(); }, [navigation]);
    return { navigation, busyTurn, historyPending: historyIntent === 'manual', historyProcessing: historyIntent !== 'none' };
}
//# sourceMappingURL=use-chat-navigation.js.map
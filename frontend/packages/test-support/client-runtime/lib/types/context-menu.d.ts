/** Gesture bindings for direct component fixtures, using the production adapter. */
import type { ContextMenuTriggerProps, MenuPoint, ContextMenuTarget } from '@deepseek-ai/dsh-client-ui-context-menu/client';
/**
 * Supply the global hook to a component mounted outside the Slot renderer.
 * @param open - Observe captured targets; an unstubbed activation throws.
 * @returns Stable hook using the production pointer and keyboard handlers.
 */
export declare function contextMenuHook<Target extends object = ContextMenuTarget>(open?: (target: Target, point: MenuPoint) => void): () => {
    getTriggerProps: (target: Target | (() => Target) | undefined, handlers?: ContextMenuTriggerProps) => ContextMenuTriggerProps;
};
//# sourceMappingURL=context-menu.d.ts.map
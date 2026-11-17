import { createContextMenuBindings } from '@deepseek-ai/dsh-client-ui-context-menu/src/client/trigger.ts';
/**
 * Supply the global hook to a component mounted outside the Slot renderer.
 * @param open - Observe captured targets; an unstubbed activation throws.
 * @returns Stable hook using the production pointer and keyboard handlers.
 */
export function contextMenuHook(open = () => { throw new Error('Unexpected context menu'); }) {
    const bindings = createContextMenuBindings(open, () => true);
    return () => bindings;
}
//# sourceMappingURL=context-menu.js.map
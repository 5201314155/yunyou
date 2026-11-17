/**
 * Extract the command line from a correlated Tool call: a shell `command`, or the text `terminal_send` types into a terminal.
 * @param call - Tool call name and arguments, when a correlated call exists.
 * @returns command text, or undefined for absent, empty, malformed, or unrelated arguments.
 */
export function commandOf(call) {
    if (call === undefined)
        return undefined;
    try {
        const args = JSON.parse(call.argsRaw);
        const command = call.name === 'terminal_send' ? args.text : args.command;
        return typeof command === 'string' && command.length > 0 ? command : undefined;
    }
    catch {
        return undefined;
    }
}
/**
 * Render the command of the Chat Tool node correlated with an approval.
 * @param props - Approval identity and Session-standard Chat selector hook.
 * @returns command text when the correlated call carries one.
 */
export function ApprovalCommand({ callId, useChat }) {
    const command = useChat((snapshot) => {
        for (const node of snapshot.nodes.values()) {
            const root = node.kind === 'tool-call' ? node.data.root : undefined;
            if (root !== undefined && root.callId === callId && !('kind' in root) && root.phase === 'start')
                return commandOf(root);
        }
        return undefined;
    });
    return command ?? null;
}
//# sourceMappingURL=ApprovalCommand.js.map
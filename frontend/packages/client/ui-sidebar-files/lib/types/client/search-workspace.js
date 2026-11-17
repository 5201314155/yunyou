/**
 * Bind the workspace search to one Remote face.
 * @param remote - the Client Remote face carrying the `workspaceFiles` namespace.
 * @returns the search the tree's face performs.
 */
export function createSearch(remote) {
    return (sessionId, request, signal) => remote.workspaceFiles.search(sessionId, request, signal);
}
//# sourceMappingURL=search-workspace.js.map
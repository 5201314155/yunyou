/** A displayed Session's header watches its terminals and opens the ones its Agent starts. */
import { useEffect } from 'react';
/**
 * Watch Agent terminals while the Session header is displayed, so its sidebar store accepts new tabs.
 * @param props - Session header lifetime and the watch owned by the plugin.
 * @returns nothing; the effect is the whole component.
 */
export function AgentTerminalOpener({ watchAgentTerminals }) {
    useEffect(() => watchAgentTerminals(), [watchAgentTerminals]);
    return null;
}
//# sourceMappingURL=AgentTerminalOpener.js.map
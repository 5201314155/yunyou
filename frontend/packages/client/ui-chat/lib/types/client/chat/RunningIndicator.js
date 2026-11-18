import { jsx as _jsx } from "react/jsx-runtime";
/** CSS breathing-dot indicator for the running Chat status (replaces the retired whale mark). */
import css from './ChatView.module.css';
/**
 * Render the decorative running dot; the CSS keyframes own the breathing timing.
 * @returns a color-inheriting pulsing dot, static under reduced motion.
 */
export function RunningIndicator() {
    return _jsx("span", { className: css.runningPulse, "aria-hidden": "true" });
}
//# sourceMappingURL=RunningIndicator.js.map
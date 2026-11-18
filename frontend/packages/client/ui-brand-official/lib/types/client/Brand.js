import { jsx as _jsx } from "react/jsx-runtime";
/** 云悠品牌主色（tokens.json semantic.color.brand.primary=云青）。 */
const BRAND_PRIMARY = '#5F87AF';
/**
 * Render the brand mark with the presentation requested by its host surface.
 * @param props - Host-supplied mark presentation.
 * @returns the yunyou cloud mark.
 */
export function OfficialBrandMark({ size }) {
    return (_jsx("span", { style: { color: BRAND_PRIMARY, fontSize: size, lineHeight: 1 }, "aria-hidden": "true", children: "\u2601" }));
}
/**
 * Render the brand name without its independently slotted mark.
 * @returns the yunyou name text.
 */
export function OfficialBrandName() {
    return (_jsx("span", { style: { color: BRAND_PRIMARY, fontWeight: 700, fontSize: 15 }, children: "\u4E91\u60A0" }));
}
//# sourceMappingURL=Brand.js.map
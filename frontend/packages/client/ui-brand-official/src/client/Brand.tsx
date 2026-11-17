// 移植自 deepseek-harness@e8f7b93，改动：品牌化批⑬B——官方 wordmark/鱼标
// 换云悠品牌：mark=☁（docs/设计/tokens.json symbol.brand），name=云悠文字，
// 色用 tokens.json brand.primary 云青 #5F87AF。
import type { SidebarBrandMarkOwnerProps } from '@deepseek-ai/dsh-client-ui-sidebar/client'

/** 云悠品牌主色（tokens.json semantic.color.brand.primary=云青）。 */
const BRAND_PRIMARY = '#5F87AF'

/**
 * Render the brand mark with the presentation requested by its host surface.
 * @param props - Host-supplied mark presentation.
 * @returns the yunyou cloud mark.
 */
export function OfficialBrandMark({ size }: SidebarBrandMarkOwnerProps) {
  return (
    <span
      style={{ color: BRAND_PRIMARY, fontSize: size, lineHeight: 1 }}
      aria-hidden="true"
    >
      ☁
    </span>
  )
}

/**
 * Render the brand name without its independently slotted mark.
 * @returns the yunyou name text.
 */
export function OfficialBrandName() {
  return (
    <span style={{ color: BRAND_PRIMARY, fontWeight: 700, fontSize: 15 }}>云悠</span>
  )
}

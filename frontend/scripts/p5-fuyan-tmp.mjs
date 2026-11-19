import { chromium } from 'playwright-core'
const url = process.argv[2], outDir = process.argv[3]
const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'], executablePath: '/home/wu/.cache/ms-playwright/chromium-1254/chrome-linux64/chrome' })
for (const d of [
  { name: 'iPhone14', viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true },
  { name: 'iPad', viewport: { width: 768, height: 1024 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
]) {
  const ctx = await browser.newContext(d)
  const page = await ctx.newPage()
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 })
  await page.waitForTimeout(10000)
  await page.emulateMedia({ features: [{ name: 'pointer', value: 'coarse' }, { name: 'hover', value: 'none' }] })
  await page.waitForTimeout(1500)
  const probe = await page.evaluate(() => {
    const inp = document.querySelector('input[class*="searchInput"]')
    const bar = inp ? inp.closest('[class]')?.parentElement : null
    const iconBtn = document.querySelector('button[class*="iconButton"]')
    const r = el => { const b = el?.getBoundingClientRect(); return b ? `${Math.round(b.width)}x${Math.round(b.height)}` : 'n/a' }
    return { searchInput: r(inp), searchInputParent: bar ? `${bar.className.toString().slice(0,40)} ${r(bar)}` : 'n/a', iconButton: r(iconBtn) }
  })
  await page.screenshot({ path: `${outDir}/P5-${d.name}.png` })
  console.log(d.name, JSON.stringify(probe))
  await ctx.close()
}
await browser.close()

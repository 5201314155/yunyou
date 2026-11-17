/**
 * 浏览器级验证脚本（云屏，任务40）：playwright-core 驱动 headless chromium，
 * 打印 root 渲染长度/body 文本/console 错误摘要+截图——移植件渲染与连接态的
 * 复验工具。用法：node scripts/verify-browser.mjs <URL带token> [截图输出路径]
 * 依赖：根 devDeps playwright-core + ~/.cache/ms-playwright 浏览器二进制。
 */
import { chromium } from 'playwright-core'
const url = process.argv[2]
const out = process.argv[3] || '/tmp/pw.png'
const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'], executablePath: '/home/wu/.cache/ms-playwright/chromium-1254/chrome-linux64/chrome' })
const page = await browser.newPage({ viewport: { width: 1280, height: 800 } })
const logs = []
page.on('console', m => logs.push(`[${m.type()}] ${m.text().slice(0, 300)}`))
page.on('pageerror', e => logs.push(`[pageerror] ${String(e).slice(0, 300)}`))
page.on('requestfailed', r => logs.push(`[reqfail] ${r.url().slice(0, 120)} ${r.failure()?.errorText}`))
await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30000 })
await page.waitForTimeout(20000)
const rootHtml = await page.evaluate(() => document.getElementById('root')?.innerHTML?.length ?? -1)
const bodyText = (await page.evaluate(() => document.body.innerText)).slice(0, 300)
await page.screenshot({ path: out })
console.log('root innerHTML 长度:', rootHtml)
console.log('body 文本:', JSON.stringify(bodyText))
for (const l of logs.slice(0, 25)) console.log(' ', l.slice(0, 220))
await browser.close()

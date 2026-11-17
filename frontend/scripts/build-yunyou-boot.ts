/**
 * 云悠 web boot 组装（移植自 deepseek-harness@e8f7b93 官方宿主 boot 链，改动：
 * entry 集裁至云悠最小致命集+inject 闭包；批次=bootstrap 单件+application 逐件；
 * 组合/渲染直接复用上游 bootInjections+renderIndexInjections 原函数，零重实现）。
 *
 * 输入：apps/web/dist（vite 壳产物）+ packages 各包 lib 目录下的 client.js（模块束）。
 * 输出：dist/index.html（注入版）+ dist/modules/<id>.js（模块束）+ dist/modules.manifest.json。
 * 运行：pnpm run build:web 后执行 `pnpm run build:boot`。
 */
import { createHash } from 'node:crypto'
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { bootInjections } from '../packages/client/modules/src/index.ts'
import { renderIndexInjections } from '../packages/host/webserver/src/injections.ts'

const root = dirname(dirname(fileURLToPath(import.meta.url)))
const distDir = join(root, 'apps/web/dist')
const modulesDir = join(distDir, 'modules')

/** 云悠最小致命集（boot-client requiredClientEntryNames 的七件；bootstrap 单列）。 */
const BOOTSTRAP_ID = '@deepseek-ai/dsh-client-modules'
const FATAL_ENTRIES = [
  '@deepseek-ai/dsh-api-session-controller',
  '@deepseek-ai/dsh-client-ui-renderer',
  '@deepseek-ai/dsh-client-ui-layout',
  '@deepseek-ai/dsh-client-ui-session',
  '@deepseek-ai/dsh-client-ui-chat',
  '@deepseek-ai/dsh-client-ui-sidebar',
] as const

interface PkgInfo { id: string; inject: string[]; bundle: string; rev: string }

/** 读一个包的 dsh.client 声明（inject 边）与 client 束 rev。 */
function pkgInfo(id: string): PkgInfo {
  const dir = pkgDir(id)
  const manifest = JSON.parse(readFileSync(join(root, dir, 'package.json'), 'utf8')) as {
    dsh?: { client?: { inject?: string[] } }
  }
  const bundle = bundlePath(id)
  if (bundle === undefined) throwUnknown(id)
  const bytes = readFileSync(bundle)
  const rev = createHash('sha256').update(bytes).digest('hex').slice(0, 16)
  return { id, inject: manifest.dsh?.client?.inject ?? [], bundle, rev }
}

/** 包名 → 仓库相对目录（全 workspace manifest 反查；构建期一次性）。 */
function pkgDir(id: string): string {
  return PKG_DIRS.get(id) ?? throwUnknown(id)
}

const PKG_DIRS = (() => {
  const map = new Map<string, string>()
  const walk = (base: string, rel: string, depth: number): void => {
    // 深度=域/包名 两级（vendor 一级），node_modules 与更深层不进
    if (depth > 2 || rel.includes('node_modules')) return
    for (const name of readdirSafe(join(root, base, rel))) {
      const sub = rel === '' ? name : `${rel}/${name}`
      try {
        const manifest = JSON.parse(readFileSync(join(root, base, sub, 'package.json'), 'utf8')) as { name?: string }
        if (manifest.name !== undefined && !map.has(manifest.name)) map.set(manifest.name, `${base}/${sub}`)
      } catch { /* 非包目录，跳过 */ }
      walk(base, sub, depth + 1)
    }
  }
  walk('packages', '', 0)
  walk('vendor', '', 0)
  return map
})()

function throwUnknown(id: string): never {
  throw new Error(`build-yunyou-boot: 包 ${id} 不在本工作区（检查 entry 集与 inject 闭包）`)
}

function readdirSafe(dir: string): string[] {
  try { return readdirRaw(dir) } catch { return [] }
}

import { readdirSync as readdirRaw } from 'node:fs'

/** inject 闭包：从七件出发收齐全部依赖包（dsh.client 声明边）。 */
function closure(): string[] {
  const seen = new Set<string>([BOOTSTRAP_ID])
  const queue = [...FATAL_ENTRIES]
  // 遍历边可经过纯服务包（无 client 束——被上游束静态吸收，不出运行时行）
  const edgeOnly = new Set<string>()
  while (queue.length > 0) {
    const id = queue.shift() as string
    seen.add(id)
    for (const dep of safeInject(id)) {
      if (!seen.has(dep) && !edgeOnly.has(dep)) { queue.push(dep) }
    }
  }
  // 拓扑序：inject 边依赖先行（与官方 graph 同律——动态包行先于请求它的行）
  const ordered: string[] = []
  const placed = new Set<string>()
  const visit = (id: string, stack: Set<string>): void => {
    if (placed.has(id) || stack.has(id)) return
    stack.add(id)
    for (const dep of safeInject(id)) visit(dep, stack)
    stack.delete(id)
    placed.add(id)
    ordered.push(id)
  }
  for (const id of seen) visit(id, new Set())
  // 出行过滤：仅「有 client 束」的包成为模块行（纯服务包被上游束吸收）
  return ordered.filter(id => hasClientBundle(id))
}

/** 无 client 束的包不取声明边（免读不存在文件）；有则读 dsh.client.inject。 */
function safeInject(id: string): string[] {
  if (!hasClientBundle(id)) return []
  return pkgInfo(id).inject
}

/** 判定包是否有 client 束（lib/client.js 或 dualClient 的 client.browser.js 在位）。 */
function hasClientBundle(id: string): boolean {
  return bundlePath(id) !== undefined
}

/** 包的浏览器束路径：标准 client.js 优先，dualClient 包退 client.browser.js。 */
function bundlePath(id: string): string | undefined {
  const dir = PKG_DIRS.get(id)
  if (dir === undefined) return undefined
  for (const name of ['lib/client.js', 'lib/client.browser.js']) {
    try {
      readFileSync(join(root, dir, name))
      return join(root, dir, name)
    } catch { /* 试下一变体 */ }
  }
  return undefined
}

function main(): void {
  const ids = closure()
  const infos = ids.map(pkgInfo)
  mkdirSync(modulesDir, { recursive: true })
  // 模块束落 dist/modules/<base64url(id)>.js：id 含 @scope/ 斜杠，URL 编码会被
  // 服务端路径解码还原成层级分隔符，与嵌入键错位——base64url 平面名无歧义
  const urlOf = (id: string): string => `./modules/${Buffer.from(id, 'utf8').toString('base64url')}.js`
  for (const info of infos) {
    copyFileSync(info.bundle, join(modulesDir, `${Buffer.from(info.id, 'utf8').toString('base64url')}.js`))
  }
  const rev = createHash('sha256').update(ids.join('\n')).digest('hex').slice(0, 16)
  const graph = {
    rev,
    release: { version: '0.1.0', commit: 'e8f7b93a344964de3dca1ace9e72b6e4d6de87cb' },
    entries: infos.map(info => ({
      id: info.id,
      url: urlOf(info.id),
      rev: info.rev,
      inject: info.inject,
      immediately: true,
      external: [],
    })),
    batches: [
      {
        phase: 'bootstrap',
        url: urlOf(BOOTSTRAP_ID),
        rev: infos.find(info => info.id === BOOTSTRAP_ID)?.rev ?? rev,
        entries: [BOOTSTRAP_ID],
      },
      ...infos.filter(info => info.id !== BOOTSTRAP_ID).map(info => ({
        phase: 'application',
        url: urlOf(info.id),
        rev: info.rev,
        entries: [info.id],
      })),
    ],
  }
  // 恒从 pristine 模板注入（vite 产物首跑时自存副本）——防重复注入叠加
  const pristine = join(distDir, 'index.pristine.html')
  try { readFileSync(pristine) } catch {
    copyFileSync(join(distDir, 'index.html'), pristine)
  }
  const indexHtml = readFileSync(pristine, 'utf8')
  const injected = renderIndexInjections(indexHtml, bootInjections(graph as never))
  writeFileSync(join(distDir, 'index.html'), injected)
  writeFileSync(join(modulesDir, '..', 'modules.manifest.json'), JSON.stringify(graph, null, 2))
  console.log(`build-yunyou-boot: ${infos.length} entries, dist/index.html 注入完成`)
}

main()

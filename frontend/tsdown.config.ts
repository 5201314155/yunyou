/**
 * 云悠 vendor 工作区 tsdown 根配置——移植自 deepseek-harness@e8f7b93
 * tsdown.config.ts，裁剪说明：本工作区只构建 client face（浏览器产物）；
 * host face（decoratorPlugin/apps/cli/apps/desktop-host）不入树，入口即拒。
 * 余参（entry 空/outDir lib/esm/es2024/dts false/clean false）随源保留。
 */
import { globSync } from 'node:fs'
import { defineConfig } from 'tsdown'
import { workspacePackageGlobs } from './scripts/workspace-package-globs.ts'

function isBuildFaceClient(value: unknown): boolean {
  if (value === undefined || value === 'host') return false
  if (value === 'client') return true
  throw new Error(`tsdown: --env.DSH_BUILD_FACE must be host or client, received ${String(value)}`)
}

export default defineConfig(({ env }) => {
  const packages = workspacePackageGlobs(globSync(
    ['vendor/*/package.json', 'packages/*/*/package.json', 'native/system/packages/*/package.json'],
    { cwd: import.meta.dirname },
  ))
  if (!isBuildFaceClient(env?.DSH_BUILD_FACE)) {
    // host face（裁剪形态）：全工作区宿主 lib（壳 vite 经 main 解析 lib/index.js）；
    // native 平台包无宿主 lib 语义，排除；decoratorPlugin 属 typert 宿主世界，不需要。
    return {
      workspace: packages.filter(p => !p.startsWith('native/')),
      entry: ['lib/types/index.js'],
      outDir: 'lib',
      format: ['esm'],
      platform: 'node',
      target: 'es2024',
      fixedExtension: false,
      dts: false,
      clean: false,
      plugins: [],
    }
  }
  return {
    workspace: [...packages],
    entry: '',
    outDir: 'lib',
    format: ['esm'],
    platform: 'node',
    target: 'es2024',
    fixedExtension: false,
    dts: false,
    clean: false,
    plugins: [],
  }
})

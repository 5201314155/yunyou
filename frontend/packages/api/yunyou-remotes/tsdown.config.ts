import { defineConfig } from 'tsdown'

/** Build the package root as a self-contained bundle (types 先行，与 schedule face 包同构）。 */
export default defineConfig([
  {
    entry: ['lib/types/index.js'],
    outDir: 'lib',
    format: ['esm'],
    platform: 'node',
    target: 'es2024',
    fixedExtension: false,
    dts: false,
    clean: false,
  },
])

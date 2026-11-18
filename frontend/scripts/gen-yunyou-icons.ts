/**
 * 云悠 web 图标生成（public/ 四张 PNG）。
 *
 * 设计：与 public/favicon.svg 同一云形语言（三圆一底的极简云）——脚本内以
 * 有符号距离场描述该形状并 4× 超采样光栅化，保证 SVG 源与 PNG 产物形状一致。
 *
 * 实现：零依赖——node 内建 zlib 做 PNG 压缩，PNG 二进制（IHDR/IDAT/IEND 与
 * CRC32）手写编码。本机无 rsvg/inkscape/ImageMagick 渲染器可用，装新依赖被
 * 任务书禁止，故程序化绘制；云形简单（圆并集），SDF 采样精度足够。
 *
 * 产物：
 *   apple-touch-icon.png   180×180  白底不透明（iOS 主屏约定）
 *   icon-192.png           192×192  透明底云青云标
 *   icon-512.png           512×512  透明底云青云标
 *   icon-maskable-512.png  512×512  云青满幅底 + 白云标（安全区 20% 内缩）
 *
 * 运行：`pnpm exec tsx scripts/gen-yunyou-icons.ts`（frontend/ 下）。
 */
import { deflateSync } from 'node:zlib'
import { writeFileSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const publicDir = join(dirname(dirname(fileURLToPath(import.meta.url))), 'apps/web/public')

/** 品牌色（tokens.json semantic.color.brand.primary 同源）。 */
const BRAND: RGB = [0x5f, 0x87, 0xaf]
/** maskable 满幅底同用云青；白云标用近白（不刺眼的暖白）。 */
const CLOUD_LIGHT: RGB = [0xe7, 0xed, 0xf2]
const WHITE: RGB = [0xff, 0xff, 0xff]

type RGB = readonly [number, number, number]

/** 云形 SDF：三圆 + 圆角底条的并集（归一化 0..1 坐标，同 favicon.svg 比例）。 */
function cloudDistance(x: number, y: number): number {
  const d1 = sdCircle(x, y, 0.345, 0.60, 0.195)
  const d2 = sdCircle(x, y, 0.516, 0.455, 0.25)
  const d3 = sdCircle(x, y, 0.705, 0.60, 0.195)
  const d4 = sdRoundRect(x, y, 0.24, 0.54, 0.78, 0.79, 0.13)
  return Math.min(d1, d2, d3, d4)
}

function sdCircle(x: number, y: number, cx: number, cy: number, r: number): number {
  return Math.hypot(x - cx, y - cy) - r
}

function sdRoundRect(x: number, y: number, x0: number, y0: number, x1: number, y1: number, r: number): number {
  const cx = (x0 + x1) / 2
  const cy = (y0 + y1) / 2
  const hx = (x1 - x0) / 2 - r
  const hy = (y1 - y0) / 2 - r
  const dx = Math.max(Math.abs(x - cx) - hx, 0)
  const dy = Math.max(Math.abs(y - cy) - hy, 0)
  return Math.hypot(dx, dy) + Math.min(Math.max(Math.abs(x - cx) - hx, Math.abs(y - cy) - hy), 0) - r
}

/** 手绘 PNG 编码：RGBA8 → PNG（无滤波，deflate 压缩）。 */
function encodePng(size: number, rgba: Uint8Array): Buffer {
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    return c >>> 0
  })
  const crc32 = (data: Uint8Array): number => {
    let c = 0xffffffff
    for (const b of data) c = crcTable[(c ^ b) & 0xff]! ^ (c >>> 8)
    return (c ^ 0xffffffff) >>> 0
  }
  const chunk = (type: string, data: Buffer): Buffer => {
    const head = Buffer.alloc(4)
    head.writeUInt32BE(data.length)
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
    const tail = Buffer.alloc(4)
    tail.writeUInt32BE(crc32(body))
    return Buffer.concat([head, body, tail])
  }
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(size, 0)
  ihdr.writeUInt32BE(size, 4)
  ihdr[8] = 8 // 位深
  ihdr[9] = 6 // RGBA
  // 每行前加滤波字节 0（None）
  const raw = Buffer.alloc(size * (size * 4 + 1))
  for (let y = 0; y < size; y++) {
    raw.set(rgba.subarray(y * size * 4, (y + 1) * size * 4), y * (size * 4 + 1) + 1)
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

interface Spec {
  file: string
  size: number
  /** 背景填充（undefined=透明底）。 */
  bg?: RGB
  /** 云形颜色。 */
  cloud: RGB
  /** 云形留白（四边占画幅比例；云形按固有纵横比在留白箱内等比缩放居中）。 */
  pad: number
  /** 抗锯齿超采样倍数。 */
  ss: number
}

/** 云形 SDF 的天然包围盒（三圆+底条并集的极值，实测常量）。 */
const CLOUD_BOX = { x0: 0.15, y0: 0.205, x1: 0.90, y1: 0.795 } as const
const CLOUD_W = CLOUD_BOX.x1 - CLOUD_BOX.x0
const CLOUD_H = CLOUD_BOX.y1 - CLOUD_BOX.y0

const SPECS: Spec[] = [
  // maskable 安全区：内容须落在中央 80% 内 → 留白 20% 起
  { file: 'apple-touch-icon.png', size: 180, bg: WHITE, cloud: BRAND, pad: 0.10, ss: 4 },
  { file: 'icon-192.png', size: 192, cloud: BRAND, pad: 0.06, ss: 4 },
  { file: 'icon-512.png', size: 512, cloud: BRAND, pad: 0.06, ss: 4 },
  { file: 'icon-maskable-512.png', size: 512, bg: BRAND, cloud: CLOUD_LIGHT, pad: 0.22, ss: 4 },
]

for (const spec of SPECS) {
  const rgba = new Uint8Array(spec.size * spec.size * 4)
  // 等比缩放：云宽=画幅-2pad，高按固有纵横比推得并垂直居中（圆不变形）
  const w = 1 - 2 * spec.pad
  const h = (w * CLOUD_H) / CLOUD_W
  const bx = spec.pad
  const by = (1 - h) / 2
  for (let py = 0; py < spec.size; py++) {
    for (let px = 0; px < spec.size; px++) {
      // 超采样：像素内 ss×ss 均匀采样，云内样本占比=alpha
      let inside = 0
      for (let sy = 0; sy < spec.ss; sy++) {
        for (let sx = 0; sx < spec.ss; sx++) {
          const tx = (px + (sx + 0.5) / spec.ss) / spec.size
          const ty = (py + (sy + 0.5) / spec.ss) / spec.size
          const x = CLOUD_BOX.x0 + (tx - bx) / w * CLOUD_W
          const y = CLOUD_BOX.y0 + (ty - by) / h * CLOUD_H
          if (cloudDistance(x, y) <= 0) inside++
        }
      }
      const alpha = inside / (spec.ss * spec.ss)
      const base = spec.bg ?? ([0, 0, 0] as const)
      const fg = spec.cloud
      const o = (py * spec.size + px) * 4
      rgba[o] = Math.round(fg[0] * alpha + base[0] * (1 - alpha))
      rgba[o + 1] = Math.round(fg[1] * alpha + base[1] * (1 - alpha))
      rgba[o + 2] = Math.round(fg[2] * alpha + base[2] * (1 - alpha))
      // 有背景时整像素不透明；透明底时 alpha 即覆盖度
      rgba[o + 3] = spec.bg ? 255 : Math.round(alpha * 255)
    }
  }
  writeFileSync(join(publicDir, spec.file), encodePng(spec.size, rgba))
  console.log(`gen-yunyou-icons: 写出 ${spec.file}（${spec.size}×${spec.size}）`)
}

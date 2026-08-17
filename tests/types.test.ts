import { describe, expect, it } from 'vitest'
import {
  cloneConfiguration,
  DEFAULT_CONFIGURATION,
  filterAndSortCaptures,
  formatBytes,
  safeSizeSlug,
  validateConfiguration,
} from '../src/types'

describe('capture viewport configuration', () => {
  it('starts with the three required enabled sizes', () => {
    expect(DEFAULT_CONFIGURATION.captures).toHaveLength(3)
    expect(DEFAULT_CONFIGURATION.captures.every((capture) => capture.enabled)).toBe(true)
    expect(DEFAULT_CONFIGURATION.captures.map((capture) => `${capture.width}x${capture.height}`)).toEqual(['1440x900', '768x1024', '390x844'])
  })

  it('rejects duplicate dimensions and a disabled queue', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    configuration.captures[1].width = 1440
    configuration.captures[1].height = 900
    expect(validateConfiguration(configuration)).toContain('キャプチャサイズの幅・高さが重複しています。')
    configuration.captures[1].width = 768
    configuration.captures[1].height = 1024
    configuration.captures.forEach((capture) => { capture.enabled = false })
    expect(validateConfiguration(configuration)).toContain('有効なキャプチャサイズを1件以上残してください。')
  })

  it('validates URL, worker and dimension ranges', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    configuration.targetUrl = 'file:///tmp/site.html'
    configuration.workers = 20
    configuration.captures[0].width = 100
    const errors = validateConfiguration(configuration)
    expect(errors).toContain('http または https で始まる有効なURLを入力してください。')
    expect(errors).toContain('同時処理数が範囲外です。')
    expect(errors).toContain('画面サイズは幅・高さとも320〜8192pxで指定してください。')
  })

  it('creates a safe deterministic size slug', () => {
    expect(safeSizeSlug({ id: 'a', label: 'My Phone / 2', width: 390, height: 844, enabled: true })).toBe('my-phone-2-390x844')
    expect(safeSizeSlug({ id: 'a', label: '...', width: 390, height: 844, enabled: true })).toBe('size-390x844')
  })
})

describe('gallery helpers', () => {
  it('formats file sizes for the compact card labels', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1024 * 840)).toBe('840 KB')
    expect(formatBytes(1024 * 1024 * 1.4)).toBe('1.4 MB')
  })

  it('filters by a started size snapshot and sorts deterministically', () => {
    const items = [
      { path: 'a', filename: 'z.png', bytes: 500, modifiedAt: 10, sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900 },
      { path: 'b', filename: 'a.png', bytes: 900, modifiedAt: 30, sizeId: 'mobile', sizeLabel: 'Mobile', width: 390, height: 844 },
      { path: 'c', filename: 'b.png', bytes: 200, modifiedAt: 20, sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900 },
    ]
    expect(filterAndSortCaptures(items, 'desktop', 'size').map((item) => item.filename)).toEqual(['z.png', 'b.png'])
    expect(filterAndSortCaptures(items, 'all', 'newest').map((item) => item.filename)).toEqual(['a.png', 'b.png', 'z.png'])
  })
})

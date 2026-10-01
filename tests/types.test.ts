import { describe, expect, it } from 'vitest'
import {
  cloneConfiguration,
  sanitizeConfigurationForStorage,
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

  it('rejects duplicate dimensions but allows metadata-only crawling', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    configuration.captures[1].width = 1440
    configuration.captures[1].height = 900
    expect(validateConfiguration(configuration)).toContain('キャプチャサイズの幅・高さが重複しています。')
    configuration.captures[1].width = 768
    configuration.captures[1].height = 1024
    configuration.captures.forEach((capture) => { capture.enabled = false })
    expect(validateConfiguration(configuration)).toEqual([])
    configuration.captures = []
    expect(validateConfiguration(configuration)).toEqual([])
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

  it('does not persist Basic auth passwords in stored configuration', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    configuration.httpAuthUser = 'staging'
    configuration.httpAuthPassword = 'secret'
    const stored = sanitizeConfigurationForStorage(configuration)
    expect(stored.httpAuthUser).toBe('staging')
    expect(stored.httpAuthPassword).toBe('')
    expect(configuration.httpAuthPassword).toBe('secret')
  })

  it('treats empty Basic auth as optional and rejects incomplete or colon usernames', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    expect(configuration.httpAuthUser).toBe('')
    expect(configuration.httpAuthPassword).toBe('')
    expect(validateConfiguration(configuration)).toEqual([])

    configuration.httpAuthPassword = 'secret'
    expect(validateConfiguration(configuration)).toContain('Basic認証のユーザー名を入力してください。')

    configuration.httpAuthUser = 'user:name'
    configuration.httpAuthPassword = 'secret'
    expect(validateConfiguration(configuration)).toContain('Basic認証のユーザー名にコロンは使えません。')

    configuration.httpAuthUser = 'user'
    configuration.httpAuthPassword = 'p:ass word'
    expect(validateConfiguration(configuration)).toEqual([])

    configuration.httpAuthUser = 'user'
    configuration.httpAuthPassword = 'secret\n'
    expect(validateConfiguration(configuration)).toContain('Basic認証に使用できない文字が含まれています。')
  })

  it('rejects non-finite, fractional and blank numeric values without coercing them', () => {
    const invalidValues: unknown[] = [NaN, Infinity, -Infinity, 1.5, '', '3']
    for (const value of invalidValues) {
      const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
      ;(configuration as unknown as { workers: unknown }).workers = value
      expect(validateConfiguration(configuration)).toContain('同時処理数が範囲外です。')
      expect((configuration as unknown as { workers: unknown }).workers).toBe(value)
    }

    const decimalViewport = cloneConfiguration(DEFAULT_CONFIGURATION)
    ;(decimalViewport.captures[0] as unknown as { width: unknown }).width = 1440.5
    expect(validateConfiguration(decimalViewport)).toContain('画面サイズは幅・高さとも320〜8192pxで指定してください。')
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


describe('Rust configuration parity', () => {
  it('limits capture ids to 80 UTF-8 bytes including surrounding whitespace', () => {
    const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
    for (const id of ['a'.repeat(81), 'あ'.repeat(27), ' ' + 'a'.repeat(80)]) {
      configuration.captures[0].id = id
      expect(validateConfiguration(configuration)).toContain('キャプチャサイズIDが不正です。')
    }
    for (const id of ['a'.repeat(80), 'あ'.repeat(26)]) {
      configuration.captures[0].id = id
      expect(validateConfiguration(configuration)).toEqual([])
    }
  })

  it('rejects every C1 control character in either Basic auth field', () => {
    for (let code = 0x80; code <= 0x9f; code += 1) {
      for (const field of ['httpAuthUser', 'httpAuthPassword'] as const) {
        const configuration = cloneConfiguration(DEFAULT_CONFIGURATION)
        configuration.httpAuthUser = 'user'
        configuration.httpAuthPassword = 'password'
        configuration[field] += String.fromCharCode(code)
        expect(validateConfiguration(configuration)).toContain('Basic認証に使用できない文字が含まれています。')
      }
    }
  })

  it.each([
    ['K', 'size'], ['İ', 'size'], ['AKİB', 'a-b'],
    ['A--B', 'a--b'], ['A- / B', 'a--b'], ['A💻💻B', 'a-b'],
    [' .ABC_-. ', 'abc_'], ['A'.repeat(81), 'a'.repeat(80)],
  ])('sanitizes %s using ASCII lowercase and Rust dash rules', (label, expected) => {
    expect(safeSizeSlug({ id: 'test', label, width: 390, height: 844, enabled: true }))
      .toBe(`${expected}-390x844`)
  })
})

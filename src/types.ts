export type ScreenshotMode = 'viewport' | 'full-page'
export type ScreenshotFormat = 'png' | 'jpg' | 'webp'
export type BrowserWait = 'networkidle' | 'load' | 'domcontentloaded'
export type RunPhase = 'idle' | 'running' | 'cancelling' | 'succeeded' | 'cancelled' | 'failed'
export type CaptureSortOrder = 'newest' | 'name' | 'size'
export type GalleryView = 'grid' | 'list'

export interface CaptureViewport {
  id: string
  label: string
  width: number
  height: number
  enabled: boolean
}

export interface CrawlConfiguration {
  targetUrl: string
  singlePage: boolean
  userAgent: string
  httpAuthUser: string
  httpAuthPassword: string
  screenshotMode: ScreenshotMode
  screenshotFormat: ScreenshotFormat
  hideCookieBanner: boolean
  maxDepth: number
  workers: number
  browserWorkers: number
  maxRequestsPerSecond: number
  browserWait: BrowserWait
  browserTimeout: number
  browserPath: string
  autoDownloadBrowser: boolean
  outputRoot: string
  captures: CaptureViewport[]
}

export interface CrawlStatus {
  phase: RunPhase
  runId: string | null
  sizeIndex: number
  sizeTotal: number
  currentSizeId: string | null
  currentSizeLabel: string | null
  currentPlan: OutputPlan | null
  plans: OutputPlan[]
  runCaptures: CaptureViewport[]
  message: string | null
}

export interface ScanRunSummary {
  runId: string
  path: string
  modifiedAt: number
  sizeCount: number
  captureCount: number
  hasHtmlReport: boolean
}

export interface OutputPlan {
  root: string
  sizeRoot: string
  captures: string
  httpCacheDir: string
  htmlReport: string
  jsonReport: string
  textReport: string
  sizeSlug: string
}

export interface CaptureItem {
  path: string
  filename: string
  bytes: number
  modifiedAt: number
  sizeId: string | null
  sizeLabel: string | null
  width: number | null
  height: number | null
}

export interface CaptureImage {
  mimeType: string
  dataBase64: string
}

export interface SeoPageItem {
  url: string
  title: string | null
  description: string | null
  canonical: string | null
  ogTitle: string | null
  ogDescription: string | null
  ogImage: string | null
  twitterTitle: string | null
  twitterDescription: string | null
  twitterImage: string | null
  h1: string | null
  h2: string | null
  sizeIds: string[]
  sizeLabels: string[]
  captureBySize?: Record<string, CaptureItem>
  status: string | null
}

export interface StartResponse {
  runId: string
  root: string
  totalSizes: number
}

export const DEFAULT_CONFIGURATION: CrawlConfiguration = {
  targetUrl: 'https://example.com',
  singlePage: false,
  userAgent: '',
  httpAuthUser: '',
  httpAuthPassword: '',
  screenshotMode: 'full-page',
  screenshotFormat: 'png',
  hideCookieBanner: true,
  maxDepth: 2,
  workers: 3,
  browserWorkers: 2,
  maxRequestsPerSecond: 5,
  browserWait: 'networkidle',
  browserTimeout: 30,
  browserPath: '',
  autoDownloadBrowser: false,
  outputRoot: '~/Pictures/MahoCrawl',
  captures: [
    { id: 'desktop', label: 'Desktop', width: 1440, height: 900, enabled: true },
    { id: 'tablet', label: 'Tablet', width: 768, height: 1024, enabled: true },
    { id: 'mobile', label: 'Mobile', width: 390, height: 844, enabled: true },
  ],
}

export function cloneConfiguration(configuration: CrawlConfiguration): CrawlConfiguration {
  return JSON.parse(JSON.stringify(configuration)) as CrawlConfiguration
}

export function sanitizeConfigurationForStorage(configuration: CrawlConfiguration): CrawlConfiguration {
  const stored = cloneConfiguration(configuration)
  stored.httpAuthPassword = ''
  return stored
}

export function safeSizeSlug(viewport: CaptureViewport): string {
  let result = ''
  let previousDash = false
  for (const character of viewport.label) {
    const lower = character.replace(/[A-Z]/g, (ascii) => ascii.toLowerCase())
    if (/^[a-z0-9._-]$/.test(lower)) {
      result += lower
      previousDash = false
    } else if (!previousDash) {
      result += '-'
      previousDash = true
    }
  }
  let value = result.replace(/^[.-]+|[.-]+$/g, '')
  if (!value) value = 'size'
  return `${value.slice(0, 80)}-${viewport.width}x${viewport.height}`
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 1024) return `${Math.max(0, bytes || 0)} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let index = 0
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024
    index += 1
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`
}

export function filterAndSortCaptures(items: CaptureItem[], filter: string, sortOrder: CaptureSortOrder): CaptureItem[] {
  const filtered = filter === 'all' ? items : items.filter((capture) => capture.sizeId === filter)
  return [...filtered].sort((left, right) => {
    if (sortOrder === 'name') return left.filename.localeCompare(right.filename, 'ja')
    if (sortOrder === 'size') return right.bytes - left.bytes
    return right.modifiedAt - left.modifiedAt
  })
}

function isIntegerInRange(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === 'number'
    && Number.isFinite(value)
    && Number.isInteger(value)
    && value >= minimum
    && value <= maximum
}

function containsDisallowedAuthChar(value: string): boolean {
  return [...value].some((character) => {
    const code = character.charCodeAt(0)
    return code <= 0x1f || (code >= 0x7f && code <= 0x9f)
  })
}

function validateHttpAuth(configuration: CrawlConfiguration): string | null {
  const user = configuration.httpAuthUser.trim()
  const password = configuration.httpAuthPassword
  if (!user && !password) return null
  if (!user) return 'Basic認証のユーザー名を入力してください。'
  if (user.includes(':')) return 'Basic認証のユーザー名にコロンは使えません。'
  if (containsDisallowedAuthChar(user) || containsDisallowedAuthChar(password)) {
    return 'Basic認証に使用できない文字が含まれています。'
  }
  return null
}

export function validateConfiguration(configuration: CrawlConfiguration): string[] {
  const errors: string[] = []
  try {
    const url = new URL(configuration.targetUrl.trim())
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) errors.push('http または https で始まる有効なURLを入力してください。')
  } catch {
    errors.push('http または https で始まる有効なURLを入力してください。')
  }
  const httpAuthError = validateHttpAuth(configuration)
  if (httpAuthError) errors.push(httpAuthError)
  if (!configuration.outputRoot.trim()) errors.push('保存先フォルダを指定してください。')
  if (!isIntegerInRange(configuration.maxDepth, 0, 20)) errors.push('クロール深度は0〜20で指定してください。')
  if (!isIntegerInRange(configuration.workers, 1, 16) || !isIntegerInRange(configuration.browserWorkers, 1, 8)) errors.push('同時処理数が範囲外です。')
  if (!isIntegerInRange(configuration.maxRequestsPerSecond, 1, 100)) errors.push('1秒あたりの最大リクエスト数は1〜100で指定してください。')
  if (!isIntegerInRange(configuration.browserTimeout, 5, 300)) errors.push('ブラウザのタイムアウトは5〜300秒で指定してください。')
  const ids = new Set<string>()
  const dimensions = new Set<string>()
  for (const viewport of configuration.captures) {
    if (!viewport.label.trim() || viewport.label.trim().length > 80) errors.push('キャプチャサイズ名は1〜80文字で指定してください。')
    if (!viewport.id.trim() || new TextEncoder().encode(viewport.id).length > 80) errors.push('キャプチャサイズIDが不正です。')
    if (ids.has(viewport.id.trim())) errors.push('キャプチャサイズIDが重複しています。')
    ids.add(viewport.id.trim())
    if (!isIntegerInRange(viewport.width, 320, 8192) || !isIntegerInRange(viewport.height, 320, 8192)) errors.push('画面サイズは幅・高さとも320〜8192pxで指定してください。')
    const dimension = `${viewport.width}x${viewport.height}`
    if (dimensions.has(dimension)) errors.push('キャプチャサイズの幅・高さが重複しています。')
    dimensions.add(dimension)
  }
  return [...new Set(errors)]
}

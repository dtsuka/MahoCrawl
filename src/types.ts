export type ScreenshotMode = 'viewport' | 'full-page'
export type ScreenshotFormat = 'png' | 'jpg' | 'webp'
export type BrowserWait = 'networkidle' | 'load' | 'domcontentloaded'
export type RunPhase = 'idle' | 'running' | 'cancelling' | 'succeeded' | 'cancelled' | 'failed'
export type CaptureSortOrder = 'newest' | 'name' | 'size'

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

export interface StartResponse {
  runId: string
  root: string
  totalSizes: number
}

export const DEFAULT_CONFIGURATION: CrawlConfiguration = {
  targetUrl: 'https://example.com',
  singlePage: false,
  userAgent: '',
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

export function safeSizeSlug(viewport: CaptureViewport): string {
  let value = viewport.label
    .trim()
    .toLocaleLowerCase('en-US')
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^[.-]+|[.-]+$/g, '')
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

export function validateConfiguration(configuration: CrawlConfiguration): string[] {
  const errors: string[] = []
  try {
    const url = new URL(configuration.targetUrl.trim())
    if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) errors.push('http または https で始まる有効なURLを入力してください。')
  } catch {
    errors.push('http または https で始まる有効なURLを入力してください。')
  }
  if (!configuration.outputRoot.trim()) errors.push('保存先フォルダを指定してください。')
  if (configuration.maxDepth < 0 || configuration.maxDepth > 20) errors.push('クロール深度は0〜20で指定してください。')
  if (configuration.workers < 1 || configuration.workers > 16 || configuration.browserWorkers < 1 || configuration.browserWorkers > 8) errors.push('同時処理数が範囲外です。')
  if (configuration.maxRequestsPerSecond < 1 || configuration.maxRequestsPerSecond > 100) errors.push('1秒あたりの最大リクエスト数は1〜100で指定してください。')
  if (configuration.browserTimeout < 5 || configuration.browserTimeout > 300) errors.push('ブラウザのタイムアウトは5〜300秒で指定してください。')
  if (configuration.captures.length === 0) errors.push('キャプチャサイズを1件以上登録してください。')
  const ids = new Set<string>()
  const dimensions = new Set<string>()
  for (const viewport of configuration.captures) {
    if (!viewport.label.trim() || viewport.label.trim().length > 80) errors.push('キャプチャサイズ名は1〜80文字で指定してください。')
    if (!viewport.id.trim()) errors.push('キャプチャサイズIDが不正です。')
    if (ids.has(viewport.id.trim())) errors.push('キャプチャサイズIDが重複しています。')
    ids.add(viewport.id.trim())
    if (viewport.width < 320 || viewport.width > 8192 || viewport.height < 320 || viewport.height > 8192) errors.push('画面サイズは幅・高さとも320〜8192pxで指定してください。')
    const dimension = `${viewport.width}x${viewport.height}`
    if (dimensions.has(dimension)) errors.push('キャプチャサイズの幅・高さが重複しています。')
    dimensions.add(dimension)
  }
  if (!configuration.captures.some((viewport) => viewport.enabled)) errors.push('有効なキャプチャサイズを1件以上残してください。')
  return [...new Set(errors)]
}

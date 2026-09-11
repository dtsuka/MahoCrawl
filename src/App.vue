<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import {
  clearLog,
  getEngineVersion,
  getLog,
  getStatus,
  isTauri,
  listCaptures,
  listSeoPages,
  loadConfiguration,
  readCapture,
  readCaptureThumbnail,
  openPath,
  revealPath,
  saveConfiguration,
  selectOutputFolder,
  startCrawl,
  stopCrawl,
  subscribeCaptures,
  subscribeOutput,
  subscribeStatus,
  validateConfigurationRust,
} from './bridge'
import {
  cloneConfiguration,
  sanitizeConfigurationForStorage,
  DEFAULT_CONFIGURATION,
  filterAndSortCaptures,
  formatBytes,
  safeSizeSlug,
  validateConfiguration,
  type CaptureItem,
  type CaptureSortOrder,
  type CaptureViewport,
  type CrawlConfiguration,
  type CrawlStatus,
  type GalleryView,
  type RunPhase,
  type SeoPageItem,
} from './types'
import { searchCaptures, searchSeoPages, sortSeoPages, type SeoPageSortOrder } from './gallery'
import SeoResultsTable from './components/SeoResultsTable.vue'
import AppIcon from './components/AppIcon.vue'

const LOCAL_STORAGE_KEY = 'maho-crawl.configuration.v1'
const GALLERY_VIEW_STORAGE_KEY = 'maho-crawl.gallery-view.v1'
const CAPTURE_PAGE_SIZE = 50
const configuration = reactive<CrawlConfiguration>(cloneConfiguration(DEFAULT_CONFIGURATION))
const status = ref<CrawlStatus>({
  phase: 'idle',
  runId: null,
  sizeIndex: 0,
  sizeTotal: 0,
  currentSizeId: null,
  currentSizeLabel: null,
  currentPlan: null,
  plans: [],
  runCaptures: [],
  message: null,
})
const captures = ref<CaptureItem[]>([])
const seoPages = ref<SeoPageItem[]>([])
const seoLoading = ref(false)
const previewUrls = reactive<Record<string, string>>({})
const thumbnailLoading = reactive(new Set<string>())
const selectedCapture = ref<CaptureItem | null>(null)
const selectedPageTitle = ref('')
const selectedPageUrl = ref('')
const selectedImageUrl = ref('')
const previewError = ref('')
const previewPending = ref(false)
const previewModal = ref<HTMLElement | null>(null)
const previewOpener = ref<HTMLElement | null>(null)
let previewRequestToken = 0
let activePreviewRequest: PreviewRequest | null = null
const activeTab = ref<'captures' | 'logs'>('captures')
const captureFilter = ref('all')
const sortOrder = ref<CaptureSortOrder | SeoPageSortOrder>('newest')
function readGalleryViewPreference(): GalleryView {
  try {
    const saved = localStorage.getItem(GALLERY_VIEW_STORAGE_KEY)
    return saved === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}
const galleryView = ref<GalleryView>(readGalleryViewPreference())
const logText = ref('')
const errorMessage = ref('')
const infoMessage = ref('')
const capturesRefreshError = ref('')
const seoRefreshError = ref('')
const refreshError = computed(() => [capturesRefreshError.value, seoRefreshError.value].filter(Boolean).join(' '))
const storageError = ref('')
const saveError = ref('')
const savingConfiguration = ref(false)
const engineVersion = ref('確認中…')
const loading = ref(true)
const starting = ref(false)
const configurationReady = ref(false)
const sidebarOpen = ref(false)
const sidebarElement = ref<HTMLElement | null>(null)
const sidebarOpener = ref<HTMLElement | null>(null)
const targetUrlInput = ref<HTMLInputElement | null>(null)
const capturePage = ref(1)
const searchInput = ref('')
const searchQuery = ref('')
const previewImage = ref<HTMLImageElement | null>(null)
const previewImageMode = ref<'fit' | 'actual'>('fit')
const previewImageLoaded = ref(false)
type SettingsSection = 'target' | 'sizes' | 'capture' | 'crawl' | 'browser' | 'output'
const sectionOpen = reactive<Record<SettingsSection, boolean>>({
  target: true,
  sizes: true,
  capture: true,
  crawl: false,
  browser: true,
  output: true,
})
const newSize = reactive({ label: '', width: 1280, height: 800 })
const unlisteners: Array<(() => void) | null> = []
let disposed = false
let lifecycleToken = 0
let initialHydration = true
let hydrationWatchPending = false
let activeRunId: string | null = null
let activeRunRoot: string | null = null
let runGeneration = 0
let startAttemptToken = 0
let saveRevision = 0
let pendingSave: { revision: number; configuration: CrawlConfiguration } | null = null
let saveQueue: Promise<void> | null = null
let capturesRefreshToken = 0
let seoRefreshToken = 0

interface PreviewRequest {
  token: number
  path: string
  pageTitle: string
  pageUrl: string
  sizeId: string | null
}

interface PreviewContext {
  pageTitle?: string
  pageUrl?: string
  sizeId?: string | null
  initialError?: string
}

interface RunSnapshot {
  runId: string
  root: string
  generation: number
}

const isBusy = computed(() => status.value.phase === 'running' || status.value.phase === 'cancelling')
const controlsDisabled = computed(() => loading.value || starting.value || isBusy.value)
const validationErrors = computed(() => validateConfiguration(configuration))
const canStart = computed(() => !controlsDisabled.value && isTauri && validationErrors.value.length === 0)
const retryableRun = computed(() => isTauri && (status.value.phase === 'failed' || status.value.phase === 'cancelled'))
const enabledSizes = computed(() => configuration.captures.filter((capture) => capture.enabled))
const metadataOnly = computed(() => enabledSizes.value.length === 0)
const runMetadataOnly = computed(() => Boolean(status.value.runId) && status.value.runCaptures.length === 0)
const gallerySizes = computed(() => status.value.runId ? status.value.runCaptures : configuration.captures)
const progressLabel = computed(() => runMetadataOnly.value
  ? 'メタ情報を取得中（キャプチャなし）'
  : `${status.value.sizeIndex} / ${status.value.sizeTotal}サイズを撮影中`)
const searchedCaptures = computed(() => searchCaptures(captures.value, seoPages.value, searchQuery.value))
const searchedSeoPages = computed(() => searchSeoPages(seoPages.value, searchQuery.value))
const filterOptions = computed(() => [
  { id: 'all', label: 'すべて', count: galleryView.value === 'grid' ? searchedCaptures.value.length : searchedSeoPages.value.length },
  ...gallerySizes.value.map((capture) => ({
    id: capture.id,
    label: capture.label,
    count: galleryView.value === 'grid'
      ? searchedCaptures.value.filter((item) => item.sizeId === capture.id).length
      : searchedSeoPages.value.filter((page) => page.sizeIds.includes(capture.id)).length,
  })),
])
const gridSortOrder = computed<CaptureSortOrder>(() => (
  sortOrder.value === 'url' || sortOrder.value === 'title' ? 'newest' : sortOrder.value
))
const rowSortOrder = computed<SeoPageSortOrder>(() => {
  if (sortOrder.value === 'name') return 'title'
  if (sortOrder.value === 'size') return 'newest'
  return sortOrder.value
})
const visibleCaptures = computed(() => {
  return filterAndSortCaptures(searchedCaptures.value, captureFilter.value, gridSortOrder.value)
})
const capturePageCount = computed(() => Math.max(1, Math.ceil(visibleCaptures.value.length / CAPTURE_PAGE_SIZE)))
const pagedVisibleCaptures = computed(() => {
  const page = Math.min(capturePage.value, capturePageCount.value)
  const start = (page - 1) * CAPTURE_PAGE_SIZE
  return visibleCaptures.value.slice(start, start + CAPTURE_PAGE_SIZE)
})
const visibleSeoPages = computed(() => {
  const filtered = captureFilter.value === 'all'
    ? searchedSeoPages.value
    : searchedSeoPages.value.filter((page) => page.sizeIds.includes(captureFilter.value))
  return sortSeoPages(filtered, rowSortOrder.value)
})
const currentProgress = computed(() => {
  if (!status.value.sizeTotal) return 0
  const completedSizes = Math.max(0, status.value.sizeIndex - 1)
  return Math.min(100, Math.round((completedSizes / status.value.sizeTotal) * 100))
})
const reportPath = computed(() => {
  const root = status.value.currentPlan?.root
  if (!root) return null
  if (captureFilter.value === 'all') return status.value.currentPlan?.htmlReport || null
  const selectedPlan = status.value.plans.find((plan) => {
    const capture = gallerySizes.value.find((item) => item.id === captureFilter.value)
    return capture ? plan.sizeSlug === safeSizeSlug(capture) : false
  })
  if (selectedPlan) return selectedPlan.htmlReport
  const selectedSize = gallerySizes.value.find((capture) => capture.id === captureFilter.value)
  return selectedSize ? `${root}/${safeSizeSlug(selectedSize)}/report.html` : status.value.currentPlan?.htmlReport || null
})

function statusLabel(phase: RunPhase): string {
  return {
    idle: '待機中',
    running: 'クロール中',
    cancelling: '停止処理中',
    succeeded: '完了',
    cancelled: '中止',
    failed: '失敗',
  }[phase]
}

function statusTone(phase: RunPhase): string {
  return {
    idle: 'neutral',
    running: 'active',
    cancelling: 'warning',
    succeeded: 'success',
    cancelled: 'warning',
    failed: 'danger',
  }[phase]
}

function statusTitle(): string {
  if (status.value.phase === 'idle') return 'クロール設定を入力してください'
  if (status.value.phase === 'running') return runMetadataOnly.value ? 'メタ情報を取得中' : status.value.currentSizeLabel ? `${status.value.currentSizeLabel}を撮影中` : 'クロールを準備しています'
  if (status.value.phase === 'cancelling') return '停止処理中'
  if (status.value.phase === 'succeeded') return 'クロール完了'
  if (status.value.phase === 'cancelled') return 'クロールを中止しました'
  if (status.value.phase === 'failed') return 'クロールに失敗しました'
  return 'クロール設定を入力してください'
}

function statusSubtitle(): string {
  if (status.value.currentPlan?.root) return status.value.currentPlan.root
  if (status.value.plans[0]?.root) return status.value.plans[0].root
  if (status.value.runId) return status.value.runId
  return 'URLを指定して、キャプチャやメタ情報を取得できます'
}

function persistLocal(): boolean {
  try {
    localStorage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(sanitizeConfigurationForStorage(configuration)))
    storageError.value = ''
    return true
  } catch (error) {
    storageError.value = `この端末に設定を保存できません。${String(error)}`
    return false
  }
}

async function flushNativeSaves(): Promise<void> {
  while (pendingSave) {
    const job = pendingSave
    pendingSave = null
    savingConfiguration.value = true
    try {
      await saveConfiguration(job.configuration)
      if (job.revision === saveRevision) saveError.value = ''
    } catch (error) {
      if (job.revision === saveRevision) saveError.value = `設定をデスクトップに保存できません。${String(error)}`
    } finally {
      savingConfiguration.value = false
    }
  }
}

function queueNativeSave(snapshot: CrawlConfiguration): void {
  if (!isTauri || loading.value || !configurationReady.value || isBusy.value || validateConfiguration(snapshot).length) return
  pendingSave = { revision: ++saveRevision, configuration: snapshot }
  if (!saveQueue) {
    saveQueue = flushNativeSaves().finally(() => { saveQueue = null })
  }
}

function persist(): void {
  if (initialHydration || loading.value || !configurationReady.value) return
  persistLocal()
  if (!isTauri || isBusy.value) return
  queueNativeSave(sanitizeConfigurationForStorage(configuration))
}

function normalizeLoaded(value: Partial<CrawlConfiguration>): void {
  const merged = { ...cloneConfiguration(DEFAULT_CONFIGURATION), ...value }
  const nextCaptures = Array.isArray(value.captures) ? value.captures : DEFAULT_CONFIGURATION.captures
  Object.assign(configuration, {
    ...merged,
    httpAuthPassword: '',
    captures: nextCaptures.map((capture, index) => ({ ...DEFAULT_CONFIGURATION.captures[index], ...capture })),
  })
}

function ensureConfigurationDefaults(): void {
  configuration.captures = configuration.captures.map((capture, index) => {
    const fallback = DEFAULT_CONFIGURATION.captures.find((item) => item.id === capture.id) || DEFAULT_CONFIGURATION.captures[index]
    return {
      ...fallback,
      ...capture,
      label: capture.label?.trim() || fallback?.label || `サイズ${index + 1}`,
      width: Number.isFinite(Number(capture.width)) && Number(capture.width) > 0 ? Number(capture.width) : fallback?.width || 1280,
      height: Number.isFinite(Number(capture.height)) && Number(capture.height) > 0 ? Number(capture.height) : fallback?.height || 800,
      enabled: capture.enabled !== false,
    }
  })
}

function isTerminalPhase(phase: RunPhase): boolean {
  return phase === 'succeeded' || phase === 'cancelled' || phase === 'failed'
}

function clearPreviewState(): void {
  invalidatePreviewRequest()
  selectedCapture.value = null
  selectedPageTitle.value = ''
  selectedPageUrl.value = ''
  selectedImageUrl.value = ''
  previewError.value = ''
  previewImageLoaded.value = false
  previewImageMode.value = 'fit'
  previewOpener.value = null
}

function resetRunArtifacts(): void {
  capturesRefreshToken += 1
  seoRefreshToken += 1
  captures.value = []
  seoPages.value = []
  seoLoading.value = false
  logText.value = ''
  capturesRefreshError.value = ''
  seoRefreshError.value = ''
  for (const key of Object.keys(previewUrls)) delete previewUrls[key]
  thumbnailLoading.clear()
  captureFilter.value = 'all'
  capturePage.value = 1
  searchInput.value = ''
  searchQuery.value = ''
  clearPreviewState()
}

function applyStatus(next: CrawlStatus): boolean {
  if (disposed) return false
  const current = status.value
  if (activeRunId && next.runId && next.runId !== activeRunId) {
    // A new run may publish its first running status before start_crawl's
    // response reaches the UI. Outside that starting window, a different run
    // id is stale and must not replace the visible run.
    if (!(starting.value && next.phase === 'running')) return false
  }
  if (activeRunId && !next.runId) return false
  if (activeRunId && next.runId === activeRunId && isTerminalPhase(current.phase) && next.phase === 'running') return false
  const nextRoot = next.currentPlan?.root || next.plans[0]?.root || null
  if (activeRunId && next.runId === activeRunId && nextRoot && activeRunRoot && nextRoot !== activeRunRoot) return false

  if (next.runId && next.runId !== activeRunId) {
    activeRunId = next.runId
    activeRunRoot = nextRoot
    runGeneration += 1
    resetRunArtifacts()
    if (next.runCaptures.length === 0) {
      galleryView.value = 'list'
      activeTab.value = 'captures'
    }
  }
  status.value = next
  if (next.runId) activeRunId = next.runId
  if (nextRoot) activeRunRoot = nextRoot
  return true
}

function currentRunSnapshot(): RunSnapshot | null {
  const runId = activeRunId || status.value.runId
  const root = status.value.currentPlan?.root || activeRunRoot || status.value.plans[0]?.root
  if (!runId || !root) return null
  return { runId, root, generation: runGeneration }
}

function isCurrentRun(snapshot: RunSnapshot): boolean {
  return !disposed
    && snapshot.runId === activeRunId
    && snapshot.root === activeRunRoot
    && snapshot.generation === runGeneration
    && status.value.runId === snapshot.runId
}

function addSize(): void {
  const label = newSize.label.trim()
  const width = newSize.width
  const height = newSize.height
  const isValidDimension = (value: unknown): value is number => (
    typeof value === 'number'
    && Number.isFinite(value)
    && Number.isInteger(value)
    && value >= 320
    && value <= 8192
  )
  if (!label || !isValidDimension(width) || !isValidDimension(height)) {
    errorMessage.value = 'サイズ名・幅・高さを入力してください。'
    return
  }
  const idBase = label.toLocaleLowerCase('en-US').replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'size'
  let id = idBase
  let suffix = 2
  while (configuration.captures.some((capture) => capture.id === id)) id = `${idBase}-${suffix++}`
  configuration.captures.push({ id, label, width, height, enabled: true })
  newSize.label = ''
  newSize.width = 1280
  newSize.height = 800
  errorMessage.value = ''
}

function removeSize(id: string): void {
  configuration.captures = configuration.captures.filter((capture) => capture.id !== id)
  if (captureFilter.value === id) captureFilter.value = 'all'
}

function toggleSection(section: SettingsSection): void {
  sectionOpen[section] = !sectionOpen[section]
}

function displayedLabel(viewport: CaptureViewport): string {
  return viewport.label || DEFAULT_CONFIGURATION.captures.find((item) => item.id === viewport.id)?.label || viewport.id
}

function displayedDimension(viewport: CaptureViewport, key: 'width' | 'height'): number | '' {
  const value = viewport[key]
  return Number.isFinite(value) ? value : ''
}

function updateLabel(viewport: CaptureViewport, event: Event): void {
  viewport.label = (event.target as HTMLInputElement).value
}

function updateDimension(viewport: CaptureViewport, key: 'width' | 'height', event: Event): void {
  const raw = (event.target as HTMLInputElement).value
  viewport[key] = raw === '' ? Number.NaN : Number(raw)
}

async function chooseOutputFolder(): Promise<void> {
  if (!isTauri) return
  try {
    const path = await selectOutputFolder()
    if (path) configuration.outputRoot = path
  } catch (error) {
    errorMessage.value = String(error)
  }
}

function applyAcceptedStart(response: { runId: string; root: string; totalSizes: number }, runCaptures: CaptureViewport[]): void {
  const sameRun = activeRunId === response.runId
  const alreadyTerminal = sameRun && status.value.runId === response.runId && isTerminalPhase(status.value.phase)
  if (alreadyTerminal) {
    activeRunRoot = response.root
    return
  }

  const current = sameRun && status.value.runId === response.runId && status.value.phase === 'running'
    ? status.value
    : {
        ...status.value,
        phase: 'running' as const,
        runId: response.runId,
        sizeIndex: 0,
        sizeTotal: response.totalSizes,
        currentSizeId: null,
        currentSizeLabel: null,
        currentPlan: null,
        plans: [],
        runCaptures,
        message: '準備中',
      }
  const applied = applyStatus({ ...current, runId: response.runId, sizeTotal: response.totalSizes })
  // start_crawl returns the output root separately from the status snapshot.
  // applyStatus remains the single place that resets a new run; this only
  // completes the accepted response's root for guarded follow-up reads.
  if (applied) activeRunRoot = response.root
}

async function beginCrawl(): Promise<void> {
  if (loading.value || starting.value || isBusy.value) return
  errorMessage.value = ''
  infoMessage.value = ''
  if (!isTauri) {
    infoMessage.value = 'ブラウザプレビューはサンプル表示のみです。実行はデスクトップアプリで行ってください。'
    return
  }
  if (validationErrors.value.length) {
    errorMessage.value = validationErrors.value[0]
    return
  }
  const token = ++startAttemptToken
  const snapshot = cloneConfiguration(configuration)
  const runCaptures = snapshot.captures.filter((capture) => capture.enabled)
  starting.value = true
  try {
    await validateConfigurationRust(snapshot)
    if (disposed || token !== startAttemptToken) return
    persistLocal()
    queueNativeSave(sanitizeConfigurationForStorage(snapshot))
    const response = await startCrawl(snapshot)
    if (disposed || token !== startAttemptToken) return
    applyAcceptedStart(response, runCaptures)
  } catch (error) {
    if (!disposed && token === startAttemptToken) errorMessage.value = String(error)
  } finally {
    if (!disposed && token === startAttemptToken) starting.value = false
  }
}

async function cancelCrawl(): Promise<void> {
  if (starting.value || !isBusy.value) return
  try {
    if (isTauri) await stopCrawl()
    else applyStatus({ ...status.value, phase: 'cancelled', message: '停止しました' })
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function refreshCaptures(snapshot = currentRunSnapshot()): Promise<void> {
  if (!isTauri || !snapshot) return
  const requestToken = ++capturesRefreshToken
  try {
    const nextCaptures = await listCaptures(snapshot.root, cloneConfiguration(configuration))
    if (!isCurrentRun(snapshot) || requestToken !== capturesRefreshToken) return
    captures.value = nextCaptures
    capturesRefreshError.value = ''
  } catch (error) {
    if (isCurrentRun(snapshot) && requestToken === capturesRefreshToken) capturesRefreshError.value = `キャプチャを更新できません。${String(error)}`
  }
}

async function refreshSeoPages(snapshot = currentRunSnapshot()): Promise<void> {
  if (!isTauri || !snapshot) return
  const requestToken = ++seoRefreshToken
  seoLoading.value = true
  try {
    const nextPages = await listSeoPages(snapshot.root)
    if (!isCurrentRun(snapshot) || requestToken !== seoRefreshToken) return
    seoPages.value = nextPages
    seoRefreshError.value = ''
  } catch (error) {
    if (isCurrentRun(snapshot) && requestToken === seoRefreshToken) seoRefreshError.value = `SEOデータを更新できません。${String(error)}`
  } finally {
    if (isCurrentRun(snapshot) && requestToken === seoRefreshToken) seoLoading.value = false
  }
}

async function retryRefresh(): Promise<void> {
  const snapshot = currentRunSnapshot()
  if (!snapshot) return
  capturesRefreshError.value = ''
  seoRefreshError.value = ''
  await Promise.all([refreshCaptures(snapshot), refreshSeoPages(snapshot)])
}

function captureKey(capture: CaptureItem): string {
  return capture.path || `${capture.filename}-${capture.sizeId || 'size'}`
}

async function loadPreviewImages(items: CaptureItem[], snapshot = currentRunSnapshot()): Promise<void> {
  if (!isTauri || !snapshot) return
  for (const capture of items) {
    if (!isCurrentRun(snapshot)) return
    const key = captureKey(capture)
    if (previewUrls[key] || thumbnailLoading.has(key)) continue
    thumbnailLoading.add(key)
    try {
      const image = await readCaptureThumbnail(capture.path)
      if (isCurrentRun(snapshot)) previewUrls[key] = `data:${image.mimeType};base64,${image.dataBase64}`
    } catch {
      // Keep the neutral placeholder for a deleted or still-being-written image.
    } finally {
      if (isCurrentRun(snapshot)) thumbnailLoading.delete(key)
    }
  }
}

function captureImage(capture: CaptureItem): string {
  return previewUrls[captureKey(capture)] || ''
}

function captureThumbnailLoading(capture: CaptureItem): boolean {
  return thumbnailLoading.has(captureKey(capture))
}

function rememberPreviewOpener(target?: EventTarget | null): void {
  const candidate = target as (HTMLElement & { focus?: () => void }) | null | undefined
  if (candidate && typeof candidate.focus === 'function') {
    previewOpener.value = candidate
  } else if (document.activeElement && 'focus' in document.activeElement && !previewModal.value?.contains(document.activeElement)) {
    previewOpener.value = document.activeElement as HTMLElement
  } else {
    previewOpener.value = null
  }
}

function isCurrentPreview(request: PreviewRequest): boolean {
  return request.token === previewRequestToken
    && activePreviewRequest === request
    && selectedCapture.value?.path === request.path
    && selectedCapture.value?.sizeId === request.sizeId
    && selectedPageTitle.value === request.pageTitle
    && selectedPageUrl.value === request.pageUrl
}

async function openCapturePreview(capture: CaptureItem, context: PreviewContext = {}, opener?: EventTarget | null): Promise<void> {
  rememberPreviewOpener(opener)
  const request: PreviewRequest = {
    token: ++previewRequestToken,
    path: capture.path,
    pageTitle: context.pageTitle || '',
    pageUrl: context.pageUrl || '',
    sizeId: context.sizeId ?? capture.sizeId,
  }
  activePreviewRequest = request
  selectedCapture.value = capture
  selectedPageTitle.value = request.pageTitle
  selectedPageUrl.value = request.pageUrl
  selectedImageUrl.value = ''
  previewError.value = context.initialError || ''
  previewPending.value = Boolean(isTauri && capture.path && !context.initialError)
  previewImageLoaded.value = false
  previewImageMode.value = 'fit'

  await nextTick()
  if (!isCurrentPreview(request)) return
  previewModal.value?.focus()
  if (!isTauri || !capture.path || context.initialError) {
    previewPending.value = false
    return
  }
  try {
    const image = await readCapture(capture.path)
    if (!isCurrentPreview(request)) return
    selectedImageUrl.value = `data:${image.mimeType};base64,${image.dataBase64}`
  } catch (error) {
    if (!isCurrentPreview(request)) return
    previewError.value = String(error)
  } finally {
    if (isCurrentPreview(request)) previewPending.value = false
  }
}

function openPreview(capture: CaptureItem, opener?: EventTarget | null): Promise<void> {
  return openCapturePreview(capture, {}, opener)
}

function pageSizeOptions(page: SeoPageItem): Array<{ id: string; label: string }> {
  return page.sizeIds.map((id, index) => ({
    id,
    label: page.captureBySize?.[id]?.sizeLabel?.trim()
      || gallerySizes.value.find((viewport) => viewport.id === id)?.label
      || (page.sizeIds.length === page.sizeLabels.length ? page.sizeLabels[index]?.trim() : '')
      || id,
  }))
}

async function openSeoCapture(page: SeoPageItem, sizeId: string, opener?: EventTarget | null): Promise<void> {
  const size = gallerySizes.value.find((viewport) => viewport.id === sizeId)
  const label = pageSizeOptions(page).find((option) => option.id === sizeId)?.label || size?.label || sizeId
  const mappedCapture = page.captureBySize?.[sizeId] || null
  const capture = mappedCapture && (!isTauri || Boolean(mappedCapture.path))
    ? {
        ...mappedCapture,
        // The mapping key is authoritative even if a malformed report carries
        // stale metadata inside the CaptureItem itself.
        sizeId,
        sizeLabel: mappedCapture.sizeLabel || label,
        width: mappedCapture.width ?? size?.width ?? null,
        height: mappedCapture.height ?? size?.height ?? null,
      }
    : null
  const fallback: CaptureItem = {
    path: '',
    filename: 'キャプチャ未取得',
    bytes: 0,
    modifiedAt: 0,
    sizeId,
    sizeLabel: label,
    width: size?.width || null,
    height: size?.height || null,
  }
  await openCapturePreview(capture || fallback, {
    pageTitle: page.title || page.url,
    pageUrl: page.url,
    sizeId,
    initialError: capture ? undefined : mappedCapture ? 'このページ・サイズの画像パスを取得できませんでした' : 'このページ・サイズのキャプチャは未取得です',
  }, opener)
}

function handlePreviewImageError(event: Event): void {
  if (!activePreviewRequest || !isCurrentPreview(activePreviewRequest)) return
  const image = event.currentTarget as HTMLImageElement | null
  if (image && image.src !== selectedImageUrl.value) return
  selectedImageUrl.value = ''
  previewPending.value = false
  previewImageLoaded.value = false
  previewImageMode.value = 'fit'
  previewError.value = '画像データを表示できません'
}

function handlePreviewImageLoad(event: Event): void {
  if (!activePreviewRequest || !isCurrentPreview(activePreviewRequest)) return
  const image = event.currentTarget as HTMLImageElement | null
  if (image && image.src !== selectedImageUrl.value) return
  previewImageLoaded.value = true
}

function retryPreview(): void {
  const capture = selectedCapture.value
  const request = activePreviewRequest
  if (!capture || !request || !capture.path || !isTauri) return
  void openCapturePreview(capture, {
    pageTitle: request.pageTitle,
    pageUrl: request.pageUrl,
    sizeId: request.sizeId,
  }, previewOpener.value)
}

function invalidatePreviewRequest(): void {
  previewRequestToken += 1
  activePreviewRequest = null
  previewPending.value = false
}

function closePreview(): void {
  const opener = previewOpener.value
  clearPreviewState()
  if (opener?.isConnected) opener.focus()
}

function onModalKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    closePreview()
    return
  }
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
    event.preventDefault()
    event.stopPropagation()
    return
  }
  if (event.key !== 'Tab') return
  event.preventDefault()
  event.stopPropagation()
  const modal = previewModal.value
  if (!modal) return
  const focusable = Array.from(modal.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])',
  ))
  if (!focusable.length) {
    modal.focus()
    return
  }
  const currentIndex = focusable.indexOf(document.activeElement as HTMLElement)
  if (currentIndex < 0) {
    focusable[event.shiftKey ? focusable.length - 1 : 0].focus()
    return
  }
  const nextIndex = event.shiftKey
    ? (currentIndex - 1 + focusable.length) % focusable.length
    : (currentIndex + 1) % focusable.length
  focusable[nextIndex].focus()
}

function focusTargetUrl(): void {
  sectionOpen.target = true
  const focus = () => { void nextTick(() => targetUrlInput.value?.focus()) }
  if (window.matchMedia?.('(max-width: 900px)').matches && !sidebarOpen.value) openSidebar()
  focus()
}

function openSidebar(target?: EventTarget | null): void {
  sidebarOpener.value = target as HTMLElement | null
  sidebarOpen.value = true
  void nextTick(() => {
    const first = sidebarElement.value?.querySelector<HTMLElement>('button, input, select, textarea, [tabindex]:not([tabindex="-1"])')
    first?.focus()
  })
}

function closeSidebar(): void {
  if (!sidebarOpen.value) return
  const opener = sidebarOpener.value
  sidebarOpen.value = false
  sidebarOpener.value = null
  void nextTick(() => opener?.focus())
}

function trapSidebarTab(event: KeyboardEvent): void {
  if (event.key !== 'Tab') return
  const sidebar = sidebarElement.value
  if (!sidebar) return
  const focusable = Array.from(sidebar.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
  ))
  if (!focusable.length) return
  const currentIndex = focusable.indexOf(document.activeElement as HTMLElement)
  if (currentIndex < 0) {
    event.preventDefault()
    focusable[0].focus()
    return
  }
  if (!event.shiftKey && currentIndex === focusable.length - 1) {
    event.preventDefault()
    focusable[0].focus()
  } else if (event.shiftKey && currentIndex === 0) {
    event.preventDefault()
    focusable[focusable.length - 1].focus()
  }
}

function applySearch(): void {
  searchQuery.value = searchInput.value.trim()
  if (!searchQuery.value) {
    infoMessage.value = ''
    return
  }
  const count = galleryView.value === 'grid' ? searchedCaptures.value.length : searchedSeoPages.value.length
  if (count === 0) {
    infoMessage.value = '一致する項目がありません。検索条件を確認するか、解除してください。'
  } else {
    infoMessage.value = ''
  }
}

function clearSearch(): void {
  searchInput.value = ''
  searchQuery.value = ''
  infoMessage.value = ''
}

function setCapturePage(page: number): void {
  capturePage.value = Math.min(capturePageCount.value, Math.max(1, page))
}

async function openReport(): Promise<void> {
  const report = reportPath.value
  if (!report || !isTauri) return
  try { await openPath(report) } catch (error) { errorMessage.value = String(error) }
}

async function openOutput(): Promise<void> {
  const root = status.value.currentPlan?.root || configuration.outputRoot
  if (!isTauri) return
  try { await openPath(root) } catch (error) { errorMessage.value = String(error) }
}

async function openCapture(capture: CaptureItem): Promise<void> {
  if (!isTauri) return
  try { await openPath(capture.path) } catch (error) { errorMessage.value = String(error) }
}

async function revealCapture(capture: CaptureItem): Promise<void> {
  if (!isTauri) return
  try { await revealPath(capture.path) } catch (error) { errorMessage.value = String(error) }
}

function capturePreview(capture: CaptureItem): string {
  if (!isTauri) {
    const hue = (capture.filename.length * 47) % 360
    return `linear-gradient(135deg, hsl(${hue} 56% 78%), hsl(${(hue + 42) % 360} 48% 92%))`
  }
  return ''
}

async function loadInitialState(): Promise<void> {
  const token = ++lifecycleToken
  let saved: string | null = null
  try {
    saved = localStorage.getItem(LOCAL_STORAGE_KEY)
  } catch (error) {
    storageError.value = `保存済み設定を読み込めません。${String(error)}`
  }
  if (saved) {
    try { normalizeLoaded(JSON.parse(saved) as Partial<CrawlConfiguration>) } catch { /* ignore malformed local storage */ }
  }
  if (disposed || token !== lifecycleToken) return
  if (isTauri) {
    try {
      const loaded = await loadConfiguration()
      if (loaded) normalizeLoaded(loaded)
    } catch { /* local storage fallback */ }
    if (disposed || token !== lifecycleToken) return
    try { engineVersion.value = await getEngineVersion() } catch { engineVersion.value = '利用不可' }
    if (disposed || token !== lifecycleToken) return
    try {
      const initialStatus = await getStatus()
      if (disposed || token !== lifecycleToken) return
      applyStatus(initialStatus)
    } catch { /* idle fallback */ }
    const logGeneration = runGeneration
    try {
      const initialLog = await getLog()
      if (!disposed && token === lifecycleToken && logGeneration === runGeneration) logText.value = initialLog
    } catch { /* empty fallback */ }
  } else {
    engineVersion.value = 'プレビュー'
    captures.value = demoCaptures()
    seoPages.value = demoSeoPages()
  }
  if (disposed || token !== lifecycleToken) return
  ensureConfigurationDefaults()
  // The deep watcher may flush after this async hydration finishes. Consume
  // that one initialization change without writing it back to the bridge.
  hydrationWatchPending = true
  configurationReady.value = true
  loading.value = false
  initialHydration = false
  const snapshot = currentRunSnapshot()
  if (isTauri && snapshot) {
    await Promise.all([refreshCaptures(snapshot), refreshSeoPages(snapshot)])
  }
}

function demoCaptures(): CaptureItem[] {
  return configuration.captures.flatMap((viewport, viewportIndex) => [0, 1].map((index) => ({
    path: '', filename: `${index === 0 ? 'home' : 'about'}-${viewport.id}.png`, bytes: 430000 + viewportIndex * 210000 + index * 38000,
    modifiedAt: Date.now() - (viewportIndex * 2 + index) * 60_000, sizeId: viewport.id, sizeLabel: viewport.label,
    width: viewport.width, height: viewport.height,
  })))
}

function demoCaptureBySize(prefix: 'home' | 'about'): Record<string, CaptureItem> {
  const result: Record<string, CaptureItem> = {}
  for (const size of configuration.captures) {
    const capture = captures.value.find((item) => item.filename === `${prefix}-${size.id}.png`)
    if (capture) result[size.id] = capture
  }
  return result
}

function demoSeoPages(): SeoPageItem[] {
  const sizes = configuration.captures.slice(0, 3)
  return [
    {
      url: 'https://example.com/',
      title: 'Example Domain',
      description: 'Example Domain のプレビュー用ページです。',
      canonical: null,
      ogTitle: 'Example Domain',
      ogDescription: 'Example Domain の紹介',
      ogImage: null,
      twitterTitle: null,
      twitterDescription: null,
      twitterImage: null,
      h1: 'Example Domain',
      h2: null,
      sizeIds: sizes.map((size) => size.id),
      sizeLabels: sizes.map((size) => size.label),
      captureBySize: demoCaptureBySize('home'),
      status: 'Allowed',
    },
    {
      url: 'https://example.com/about',
      title: 'About Example',
      description: null,
      canonical: null,
      ogTitle: null,
      ogDescription: null,
      ogImage: null,
      twitterTitle: null,
      twitterDescription: null,
      twitterImage: null,
      h1: 'About Example',
      h2: 'Details',
      sizeIds: sizes.slice(0, 1).map((size) => size.id),
      sizeLabels: sizes.slice(0, 1).map((size) => size.label),
      captureBySize: demoCaptureBySize('about'),
      status: 'Allowed',
    },
  ]
}

function applyStatusEvent(next: CrawlStatus): void {
  if (!applyStatus(next)) return
  const snapshot = currentRunSnapshot()
  if (snapshot) {
    void refreshCaptures(snapshot)
    void refreshSeoPages(snapshot)
  }
}

function applyCaptureEvent(_next: CaptureItem[]): void {
  const snapshot = currentRunSnapshot()
  if (snapshot) void refreshCaptures(snapshot)
}

function appendOutputEvent(text: string): void {
  if (disposed || !activeRunId || status.value.phase === 'idle') return
  logText.value += text
  if (logText.value.length > 400_000) logText.value = logText.value.slice(-400_000)
}

function onKeydown(event: KeyboardEvent): void {
  if (sidebarOpen.value && !selectedCapture.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      closeSidebar()
      return
    }
    trapSidebarTab(event)
    return
  }
  if (selectedCapture.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      closePreview()
    } else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault()
    }
    return
  }
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && canStart.value) {
    event.preventDefault()
    void beginCrawl()
  }
}

watch(configuration, () => {
  if (initialHydration) return
  if (hydrationWatchPending) {
    hydrationWatchPending = false
    return
  }
  void persist()
}, { deep: true })
watch(galleryView, (value) => {
  try { localStorage.setItem(GALLERY_VIEW_STORAGE_KEY, value) } catch { /* preference is optional */ }
})
watch([captureFilter, sortOrder, searchQuery], () => { capturePage.value = 1 })
watch(capturePageCount, (count) => {
  if (capturePage.value > count) capturePage.value = count
})
watch(pagedVisibleCaptures, (items) => {
  const snapshot = currentRunSnapshot()
  if (snapshot) void loadPreviewImages(items, snapshot)
})
watch(galleryView, (value) => {
  if (value === 'grid') {
    if (sortOrder.value === 'url' || sortOrder.value === 'title') sortOrder.value = 'newest'
  } else if (sortOrder.value === 'name') {
    sortOrder.value = 'title'
  } else if (sortOrder.value === 'size') {
    sortOrder.value = 'newest'
  }
})

onMounted(async () => {
  disposed = false
  window.addEventListener('keydown', onKeydown)
  if (isTauri) {
    try { unlisteners.push(await subscribeStatus(applyStatusEvent)) } catch (error) { errorMessage.value = `状態通知を購読できません。${String(error)}` }
    try { unlisteners.push(await subscribeOutput(appendOutputEvent)) } catch (error) { errorMessage.value = `ログ通知を購読できません。${String(error)}` }
    try { unlisteners.push(await subscribeCaptures(applyCaptureEvent)) } catch (error) { errorMessage.value = `キャプチャ通知を購読できません。${String(error)}` }
  }
  await loadInitialState()
})

onBeforeUnmount(() => {
  disposed = true
  lifecycleToken += 1
  startAttemptToken += 1
  invalidatePreviewRequest()
  window.removeEventListener('keydown', onKeydown)
  unlisteners.forEach((unlisten) => unlisten?.())
})
</script>

<template>
  <main class="app-shell" :class="{ 'is-busy': isBusy }">
    <aside ref="sidebarElement" class="settings-sidebar" :class="{ 'sidebar-open': sidebarOpen }" aria-label="クロール設定">
      <div class="settings-scroll">
        <section class="setting-section">
          <h2 class="section-heading" id="target-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.target" aria-controls="target-settings" @click="toggleSection('target')">
              <span class="section-title">クロール対象</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="target-settings" v-show="sectionOpen.target" class="section-content">
            <label class="field-label" for="target-url">URL</label>
            <input id="target-url" ref="targetUrlInput" v-model="configuration.targetUrl" class="text-input" type="url" placeholder="https://example.com" :disabled="controlsDisabled" />
            <label class="switch-row">
              <input v-model="configuration.singlePage" type="checkbox" :disabled="controlsDisabled" />
              <span class="switch" aria-hidden="true"></span><span>このページのみ</span>
            </label>
            <label class="field-label" for="user-agent">ユーザーエージェント（カスタム）</label>
            <input id="user-agent" v-model="configuration.userAgent" class="text-input" type="text" placeholder="空欄ならDesktop" :disabled="controlsDisabled" />
            <div class="field-grid auth-fields">
              <label class="field-label" for="http-auth-user">Basic認証（ユーザー名）
                <input id="http-auth-user" v-model="configuration.httpAuthUser" class="text-input" type="text" autocomplete="off" spellcheck="false" placeholder="任意" aria-describedby="http-auth-note" :disabled="controlsDisabled" />
              </label>
              <label class="field-label" for="http-auth-password">パスワード
                <input id="http-auth-password" v-model="configuration.httpAuthPassword" class="text-input" type="password" autocomplete="off" placeholder="任意" aria-describedby="http-auth-note" :disabled="controlsDisabled" />
              </label>
            </div>
            <p id="http-auth-note" class="path-note">空欄なら認証なし。パスワードは保存せず、実行時だけ対象URLのoriginへ渡します。</p>
          </div>
        </section>

        <section class="setting-section sizes-section">
          <h2 class="section-heading" id="capture-size-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.sizes" aria-controls="capture-size-settings" @click="toggleSection('sizes')">
              <span class="section-title">キャプチャサイズ</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="capture-size-settings" v-show="sectionOpen.sizes" class="section-content">
            <p class="section-help">有効なサイズを順番に撮影します。すべてオフ、または削除するとメタ情報のみ取得します。</p>
            <p v-if="metadataOnly" class="field-note" role="status">キャプチャなしでHTML内のメタ情報を取得します。JavaScriptは実行しません。</p>
            <div v-for="viewport in configuration.captures" :key="viewport.id" class="size-row">
              <label class="checkbox-label" :title="`${viewport.label}を有効化`">
                <input v-model="viewport.enabled" type="checkbox" :disabled="controlsDisabled" :aria-label="`${displayedLabel(viewport)}を有効化`" />
                <span class="checkmark" aria-hidden="true"><AppIcon name="check" /></span>
              </label>
              <div class="size-fields">
                <input :value="viewport.label" @input="updateLabel(viewport, $event)" class="size-label-input" :disabled="controlsDisabled" :aria-label="`${displayedLabel(viewport)}の名前`" />
                <div class="dimension-fields">
                <input :value="displayedDimension(viewport, 'width')" @input="updateDimension(viewport, 'width', $event)" class="dimension-input" type="number" min="320" max="8192" :disabled="controlsDisabled" :aria-label="`${displayedLabel(viewport)}の幅`" />
                <span>×</span>
                  <input :value="displayedDimension(viewport, 'height')" @input="updateDimension(viewport, 'height', $event)" class="dimension-input" type="number" min="320" max="8192" :disabled="controlsDisabled" :aria-label="`${displayedLabel(viewport)}の高さ`" />
                </div>
              </div>
              <button class="icon-button delete" type="button" :disabled="controlsDisabled" :aria-label="`${displayedLabel(viewport)}を削除`" @click="removeSize(viewport.id)"><AppIcon name="close" /></button>
            </div>
            <div class="add-size-row">
              <input v-model="newSize.label" class="size-label-input" type="text" placeholder="新しいサイズ" aria-label="新しいサイズ名" :disabled="controlsDisabled" @keydown.enter="addSize" />
              <input v-model.number="newSize.width" class="dimension-input" type="number" min="320" max="8192" aria-label="新しいサイズの幅" :disabled="controlsDisabled" />
              <span>×</span>
              <input v-model.number="newSize.height" class="dimension-input" type="number" min="320" max="8192" aria-label="新しいサイズの高さ" :disabled="controlsDisabled" />
              <button class="add-button" type="button" :disabled="controlsDisabled" @click="addSize"><AppIcon name="plus" />追加</button>
            </div>
          </div>
        </section>

        <section class="setting-section">
          <h2 class="section-heading" id="capture-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.capture" aria-controls="capture-settings" @click="toggleSection('capture')">
              <span class="section-title">撮影設定</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="capture-settings" v-show="sectionOpen.capture" class="section-content">
            <div class="field-grid">
              <label class="field-label">撮影モード
                <select v-model="configuration.screenshotMode" class="select-input" :disabled="controlsDisabled || metadataOnly"><option value="viewport">表示領域</option><option value="full-page">ページ全体</option></select>
              </label>
              <label class="field-label">出力フォーマット
                <select v-model="configuration.screenshotFormat" class="select-input" :disabled="controlsDisabled || metadataOnly"><option value="png">PNG</option><option value="jpg">JPG</option><option value="webp">WebP</option></select>
              </label>
            </div>
            <label class="switch-row">
              <input v-model="configuration.hideCookieBanner" type="checkbox" disabled aria-describedby="cookie-banner-note" />
              <span class="switch" aria-hidden="true"></span><span>Cookieバナー自動非表示（現在未対応）</span>
            </label>
            <p id="cookie-banner-note" class="field-note">設定値は保存されますが、現在のCLIでは自動非表示を実行しません。</p>
          </div>
        </section>

        <section class="setting-section">
          <h2 class="section-heading" id="crawl-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.crawl" aria-controls="crawl-settings" @click="toggleSection('crawl')">
              <span class="section-title">クロール設定</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="crawl-settings" v-show="sectionOpen.crawl" class="section-content advanced-fields">
            <label class="field-label">最大深度<input v-model.number="configuration.maxDepth" class="number-input" type="number" min="0" max="20" :disabled="controlsDisabled || configuration.singlePage" /></label>
            <label class="field-label">同時ワーカー数<input v-model.number="configuration.workers" class="number-input" type="number" min="1" max="16" :disabled="controlsDisabled" /></label>
            <label class="field-label">ブラウザワーカー数<input v-model.number="configuration.browserWorkers" class="number-input" type="number" min="1" max="8" :disabled="controlsDisabled || metadataOnly" /></label>
            <label class="field-label">最大リクエスト / 秒<input v-model.number="configuration.maxRequestsPerSecond" class="number-input" type="number" min="1" max="100" :disabled="controlsDisabled" /></label>
          </div>
        </section>

        <section class="setting-section">
          <h2 class="section-heading" id="browser-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.browser" aria-controls="browser-settings" @click="toggleSection('browser')">
              <span class="section-title">ブラウザ</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="browser-settings" v-show="sectionOpen.browser" class="section-content">
            <div class="field-grid">
              <label class="field-label">待機条件<select v-model="configuration.browserWait" class="select-input" :disabled="controlsDisabled || metadataOnly"><option value="networkidle">通信完了まで</option><option value="load">loadイベントまで</option><option value="domcontentloaded">DOM構築まで</option></select></label>
              <label class="field-label">タイムアウト（秒）<input v-model.number="configuration.browserTimeout" class="number-input" min="5" max="300" type="number" :disabled="controlsDisabled || metadataOnly" /></label>
            </div>
            <label class="field-label">ブラウザのパス<input v-model="configuration.browserPath" class="text-input" type="text" placeholder="自動検出" :disabled="controlsDisabled || metadataOnly" /></label>
            <label class="switch-row"><input v-model="configuration.autoDownloadBrowser" type="checkbox" :disabled="controlsDisabled || metadataOnly" /><span class="switch" aria-hidden="true"></span><span>見つからない場合に自動取得</span></label>
          </div>
        </section>

        <section class="setting-section output-section">
          <h2 class="section-heading" id="output-settings-heading">
            <button class="section-toggle" type="button" :aria-expanded="sectionOpen.output" aria-controls="output-settings" @click="toggleSection('output')">
              <span class="section-title">保存先</span>
              <span class="section-chevron" aria-hidden="true"></span>
            </button>
          </h2>
          <div id="output-settings" v-show="sectionOpen.output" class="section-content">
            <div class="output-picker"><input v-model="configuration.outputRoot" class="text-input" type="text" aria-label="保存先フォルダ" :disabled="controlsDisabled" /><button type="button" :disabled="controlsDisabled || !isTauri" @click="chooseOutputFolder">参照…</button></div>
            <p class="path-note">実行ごとに日時フォルダ、サイズごとに専用フォルダを作成します。</p>
          </div>
        </section>
      </div>

      <div class="sidebar-footer">
        <button v-if="isBusy" class="primary-button stop-button" type="button" @click="cancelCrawl"><AppIcon name="stop" /> クロールを中止</button>
        <button v-else class="primary-button" type="button" :disabled="!canStart" @click="beginCrawl"><AppIcon name="play" /> {{ starting ? '準備しています…' : retryableRun ? 'もう一度実行' : 'クロールを開始' }}</button>
        <p v-if="!isTauri" class="run-note">ブラウザプレビュー・サンプルデータ。実行はデスクトップアプリで行います。</p>
        <p v-if="savingConfiguration" class="save-note" role="status">設定を保存しています…</p>
        <p v-if="storageError || saveError" class="save-note error" role="alert">{{ storageError || saveError }}</p>
        <div class="engine-line"><span class="engine-dot" :class="{ online: engineVersion !== '利用不可' }"></span> Engine: {{ engineVersion }}</div>
        <div class="license-line">SiteOne Crawler (MIT) · 非公式ラッパー</div>
      </div>
    </aside>
    <div v-if="sidebarOpen" class="sidebar-scrim" aria-hidden="true" @click="closeSidebar"></div>

    <section class="workspace" aria-label="キャプチャワークスペース">
      <header class="run-header">
        <button class="sidebar-toggle" type="button" aria-label="設定を表示" :aria-expanded="sidebarOpen" @click="openSidebar($event.currentTarget)">
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.25h11M2.5 8h11M2.5 11.75h11" /></svg>
        </button>
        <div class="phase-icon" :class="statusTone(status.phase)" aria-hidden="true"><span v-if="isBusy" class="spinner"></span><AppIcon v-else-if="status.phase === 'succeeded'" name="check" /><AppIcon v-else-if="status.phase === 'failed'" name="alert" /><span v-else class="phase-ring"></span></div>
        <div class="run-title"><h2>{{ statusTitle() }}</h2><p>{{ statusSubtitle() }}</p></div>
        <div class="run-actions"><button type="button" :disabled="!reportPath || !isTauri" @click="openReport"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 2.5h7l3 3v8H3zM10 2.5v3h3M5.5 8h5M5.5 10.5h5" /></svg><span>HTMLレポート</span></button><button type="button" :disabled="!status.runId || !isTauri" @click="openOutput"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 4.5h4l1.25 1.5h5.75v7.5h-11zM2.5 4.5v-1h4l1.25 1" /></svg><span>保存先</span></button></div>
      </header>
      <div v-if="isBusy" class="progress-strip" role="progressbar" :aria-valuenow="runMetadataOnly ? undefined : currentProgress" :aria-valuemin="0" :aria-valuemax="100" :aria-label="progressLabel"><span :style="{ transform: `scaleX(${currentProgress / 100})` }"></span></div>
      <div v-if="isBusy" class="progress-label">{{ progressLabel }}<span v-if="status.currentSizeLabel"> · {{ status.currentSizeLabel }}</span></div>
      <div v-if="errorMessage || infoMessage || refreshError || storageError || saveError || validationErrors.length || (status.phase === 'failed' && status.message)" class="message-area" role="status">
        <p v-if="errorMessage" class="message error">{{ errorMessage }}</p>
        <p v-else-if="status.phase === 'failed' && status.message" class="message error">{{ status.message }}</p>
        <p v-else-if="validationErrors.length && !isBusy" class="message error">{{ validationErrors[0] }}</p>
        <p v-if="infoMessage" class="message info">{{ infoMessage }}</p>
        <div v-if="refreshError" class="message error refresh-notice" role="alert"><span>{{ refreshError }}</span><button type="button" @click="retryRefresh">再試行</button></div>
      </div>

      <nav class="tabs" aria-label="表示切替"><button type="button" :class="{ active: activeTab === 'captures' }" @click="activeTab = 'captures'">{{ runMetadataOnly ? 'メタ情報' : 'キャプチャ' }} <span v-if="runMetadataOnly ? seoPages.length : captures.length">{{ runMetadataOnly ? seoPages.length : captures.length }}</span></button><button type="button" :class="{ active: activeTab === 'logs' }" @click="activeTab = 'logs'">ログ</button></nav>

      <div v-if="activeTab === 'captures'" class="gallery-workspace">
        <div class="gallery-toolbar">
          <div class="filter-pills"><button v-for="option in filterOptions" :key="option.id" type="button" :class="{ active: captureFilter === option.id }" :aria-pressed="captureFilter === option.id" @click="captureFilter = option.id">{{ option.label }} <span>{{ option.count }}</span></button></div>
          <form class="gallery-search" role="search" @submit.prevent="applySearch">
            <label class="sr-only" for="gallery-search-input">キャプチャとページを検索</label>
            <input id="gallery-search-input" v-model="searchInput" type="search" placeholder="URL・タイトル・ファイル名を検索" autocomplete="off" />
            <button type="submit" aria-label="検索"><svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="7" cy="7" r="4.25" /><path d="m10.25 10.25 3 3" /></svg></button>
            <button v-if="searchInput || searchQuery" class="search-clear" type="button" aria-label="検索を解除" @click="clearSearch"><AppIcon name="close" /></button>
          </form>
          <div class="sort-tools"><label for="sort-order">並び替え:</label><select id="sort-order" v-model="sortOrder" class="sort-select"><template v-if="galleryView === 'grid'"><option value="newest">取得日時（新しい順）</option><option value="name">ファイル名</option><option value="size">ファイルサイズ</option></template><template v-else><option value="url">URL</option><option value="title">タイトル</option><option value="newest">取得日時（新しい順）</option></template></select><div class="view-mode-toggle" role="group" aria-label="ギャラリー表示"><button class="view-toggle" :class="{ active: galleryView === 'grid' }" type="button" :aria-pressed="galleryView === 'grid'" aria-label="グリッド表示" @click="galleryView = 'grid'"><AppIcon name="grid" /><span class="view-toggle-text">グリッド</span></button><button class="view-toggle" :class="{ active: galleryView === 'list' }" type="button" :aria-pressed="galleryView === 'list'" aria-label="行表示" @click="galleryView = 'list'"><AppIcon name="list" /><span class="view-toggle-text">行</span></button></div></div>
        </div>
        <div v-if="loading" class="empty-state"><span class="empty-icon" aria-hidden="true"><span class="spinner"></span></span><h3>準備しています</h3><p>設定を読み込んでいます。</p></div>
        <div v-else-if="galleryView === 'grid' && runMetadataOnly" class="empty-state"><span class="empty-icon"><AppIcon name="list" /></span><h3>キャプチャなしで実行しています</h3><p>取得したメタ情報は行表示で確認できます。</p><button type="button" class="empty-action" @click="galleryView = 'list'">メタ情報を表示</button></div>
        <div v-else-if="galleryView === 'grid' && visibleCaptures.length === 0" class="empty-state"><span class="empty-icon"><AppIcon name="image" /></span><h3>{{ searchQuery ? '検索結果がありません' : isBusy ? 'キャプチャを待っています' : 'キャプチャはまだありません' }}</h3><p v-if="searchQuery">URL・タイトル・ファイル名を確認してください。</p><p v-else>{{ isBusy ? '撮影されたページから順に表示します。' : isTauri ? '左側で設定し、クロールを開始してください。' : 'ここではサンプルデータを表示します。実行はデスクトップアプリで行ってください。' }}</p><button v-if="searchQuery" type="button" class="empty-action" @click="clearSearch">検索を解除</button><button v-else-if="isTauri && !isBusy" type="button" class="empty-action" @click="focusTargetUrl">URLを入力する</button></div>
        <div v-else-if="galleryView === 'list' && seoLoading && seoPages.length === 0" class="empty-state"><span class="empty-icon" aria-hidden="true"><span class="spinner"></span></span><h3>SEOデータを読み込んでいます</h3><p>レポートからページ情報を整理しています。</p></div>
        <div v-else-if="galleryView === 'list' && visibleSeoPages.length === 0" class="empty-state"><span class="empty-icon"><AppIcon name="list" /></span><h3>{{ searchQuery ? '検索結果がありません' : 'SEOデータは未取得です' }}</h3><p>{{ searchQuery ? 'URL・タイトル・SEO項目を確認してください。' : 'クロール完了後にSiteOneのJSONレポートから表示します。' }}</p><button v-if="searchQuery" type="button" class="empty-action" @click="clearSearch">検索を解除</button><button v-else-if="isTauri && !isBusy" type="button" class="empty-action" @click="focusTargetUrl">URLを入力する</button></div>
        <div v-else-if="galleryView === 'grid'" class="capture-grid">
          <article v-for="capture in pagedVisibleCaptures" :key="capture.path || `${capture.filename}-${capture.sizeId || 'size'}`" class="capture-card" tabindex="0" @click="openPreview(capture, $event.currentTarget)" @dblclick="openCapture(capture)" @keydown.self.enter.prevent="openPreview(capture, $event.currentTarget)" @keydown.self.space.prevent="openPreview(capture, $event.currentTarget)">
            <div class="capture-thumb" :style="{ background: capturePreview(capture) }"><img v-if="captureImage(capture)" :src="captureImage(capture)" :alt="`${capture.filename}のプレビュー`" /><div v-else-if="!isTauri" class="mock-page"><span></span><i></i><b></b><em></em></div><div v-else-if="captureThumbnailLoading(capture)" class="capture-thumb-state" role="status">画像を読み込み中…</div><div v-else class="capture-thumb-state">画像未取得</div></div>
            <div class="capture-meta"><div class="capture-file"><AppIcon class="device-icon" :name="capture.width && capture.width > 1000 ? 'desktop' : 'mobile'" /><span class="truncate">{{ capture.filename }}</span></div><div class="capture-tags"><span class="size-tag" :class="capture.sizeId">{{ capture.sizeLabel || 'サイズ' }}</span><span>{{ capture.width }} × {{ capture.height }}</span></div><div class="capture-bottom"><span>{{ formatBytes(capture.bytes) }}</span><span class="capture-actions"><button type="button" @click.stop="openCapture(capture)" :disabled="!isTauri" aria-label="元画像を開く"><AppIcon name="external" /></button><button type="button" @click.stop="revealCapture(capture)" :disabled="!isTauri" aria-label="Finderで表示"><AppIcon name="folder" /></button></span></div></div>
          </article>
        </div>
        <SeoResultsTable v-else :pages="visibleSeoPages" :sizes="gallerySizes" @open-capture="openSeoCapture" />
        <footer class="gallery-footer"><template v-if="galleryView === 'grid'"><span>{{ visibleCaptures.length }}件のキャプチャ</span><span v-if="enabledSizes.length"> · {{ enabledSizes.length }}サイズを順次処理</span><nav v-if="capturePageCount > 1" class="pagination" aria-label="キャプチャページ"><button type="button" :disabled="capturePage <= 1" aria-label="前のページ" @click="setCapturePage(capturePage - 1)"><AppIcon name="previous" /></button><span>{{ capturePage }} / {{ capturePageCount }}</span><button type="button" :disabled="capturePage >= capturePageCount" aria-label="次のページ" @click="setCapturePage(capturePage + 1)"><AppIcon name="next" /></button></nav></template><template v-else>{{ visibleSeoPages.length }}件のページ · SEO概要</template></footer>
      </div>
      <div v-else class="log-workspace"><div class="log-toolbar"><span>実行ログ</span><button type="button" :disabled="!logText" @click="clearLog().then(() => { logText = '' })">ログを消去</button></div><pre>{{ logText || 'クロールを開始するとログが表示されます。' }}</pre></div>
    </section>
    <div v-if="selectedCapture" class="preview-backdrop" role="presentation" @click.self="closePreview">
      <section ref="previewModal" class="preview-modal" role="dialog" aria-modal="true" tabindex="-1" :aria-label="`${selectedPageTitle || selectedCapture.filename}のプレビュー`" @keydown="onModalKeydown">
        <header><div><h2>{{ selectedPageTitle || selectedCapture.filename }}</h2><p v-if="selectedPageUrl" class="preview-url" :title="selectedPageUrl">{{ selectedPageUrl }}</p><p class="preview-meta"><span class="preview-filename" :title="selectedCapture.filename">{{ selectedCapture.filename }}</span><span class="preview-size"> · {{ selectedCapture.sizeLabel || 'サイズ未特定' }} · {{ selectedCapture.width || '—' }} × {{ selectedCapture.height || '—' }}</span><span class="preview-bytes"> · {{ formatBytes(selectedCapture.bytes) }}</span></p></div><button type="button" aria-label="プレビューを閉じる" @click="closePreview"><AppIcon name="close" /></button></header>
        <div v-if="selectedImageUrl || previewPending || previewError" class="preview-controls" aria-label="画像表示"><span>表示:</span><button type="button" :disabled="!previewImageLoaded || !!previewError" :class="{ active: previewImageMode === 'fit' }" :aria-pressed="previewImageMode === 'fit'" @click="previewImageMode = 'fit'">全体</button><button type="button" :disabled="!previewImageLoaded || !!previewError" :class="{ active: previewImageMode === 'actual' }" :aria-pressed="previewImageMode === 'actual'" @click="previewImageMode = 'actual'">100%</button><span v-if="previewImageLoaded && previewImage" class="preview-natural-size">{{ previewImage.naturalWidth }} × {{ previewImage.naturalHeight }}px</span></div>
        <div class="preview-image-wrap" :class="{ 'is-actual': previewImageMode === 'actual' }"><div v-if="previewPending" class="preview-loading" role="status" aria-live="polite">読み込み中…</div><img v-else-if="selectedImageUrl" ref="previewImage" :src="selectedImageUrl" :alt="`${selectedCapture.filename}のプレビュー`" @load="handlePreviewImageLoad" @error="handlePreviewImageError" /><div v-else-if="previewError" class="preview-error"><strong>プレビューを読み込めませんでした</strong><span>{{ previewError }}</span><small>元画像を開くか、Finderで表示してください。</small><button v-if="selectedCapture.path" type="button" @click="retryPreview">再試行</button></div><div v-else-if="!isTauri" class="mock-page large"><span></span><i></i><b></b><em></em></div><div v-else class="preview-error"><strong>プレビュー画像がありません</strong><span>このキャプチャには表示可能な画像がありません。</span></div></div>
        <footer><button type="button" :disabled="!isTauri || !selectedCapture.path" @click="openCapture(selectedCapture)">元画像を開く</button><button type="button" :disabled="!isTauri || !selectedCapture.path" @click="revealCapture(selectedCapture)">Finderで表示</button></footer>
      </section>
    </div>
  </main>
</template>

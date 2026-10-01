<script setup lang="ts">
import type { SettingsSection } from './composables/contracts'
import { useConfigurationPersistence } from './composables/useConfigurationPersistence'
import { useCrawlRun } from './composables/useCrawlRun'
import { usePreview } from './composables/usePreview'
import { useHistory } from './composables/useHistory'

import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { clearLog, getEngineVersion, getLog, getStatus, isTauri, loadConfiguration, openPath, openExternalUrl, revealPath, subscribeCaptures, subscribeOutput, subscribeStatus } from './bridge'
import { cloneConfiguration, DEFAULT_CONFIGURATION, filterAndSortCaptures, safeSizeSlug, validateConfiguration, type CaptureItem, type CaptureSortOrder, type CrawlConfiguration, type CrawlStatus, type GalleryView, type RunPhase, type SeoPageItem } from './types'
import { buildCapturePageIndex, searchCaptures, searchSeoPages, sortSeoPages, type SeoPageSortOrder } from './gallery'
import SeoResultsTable from './components/SeoResultsTable.vue'
import AppIcon from './components/AppIcon.vue'
import SettingsSidebar from './components/SettingsSidebar.vue'
import CaptureGrid from './components/CaptureGrid.vue'
import HistoryModal from './components/HistoryModal.vue'
import PreviewModal from './components/PreviewModal.vue'

import { useOutputBuffer } from './composables/useOutputBuffer'

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
const galleryWorkspace = ref<HTMLElement | null>(null)
const activeCapture = ref<CaptureItem | null>(null)
const activePageUrl = ref('')
const activeSizeId = ref<string | null>(null)
const capturePageIndex = computed(() => buildCapturePageIndex(seoPages.value))

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
const outputBuffer = useOutputBuffer(computed(() => activeTab.value === 'logs'))
const { logText } = outputBuffer
const errorMessage = ref('')
const infoMessage = ref('')
const capturesRefreshError = ref('')
const seoRefreshError = ref('')
const refreshError = computed(() => [capturesRefreshError.value, seoRefreshError.value].filter(Boolean).join(' '))

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

const sectionOpen = reactive<Record<SettingsSection, boolean>>({
  target: true,
  sizes: true,
  capture: true,
  crawl: false,
  browser: true,
  output: true,
})

const unlisteners: Array<(() => void) | null> = []
let disposed = false
let lifecycleToken = 0

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

const {
  hydrateLocal,
  persistLocal,
  queueNativeSave,
  normalizeLoaded,
  ensureConfigurationDefaults,
  completeHydration,
  storageError,
  saveError,
  savingConfiguration,
} = useConfigurationPersistence({
  configuration,
  loading,
  configurationReady,
  isBusy,
})

const {
  resetRunArtifacts,
  applyStatus,
  currentRunSnapshot,
  beginCrawl,
  cancelCrawl,
  refreshCaptures,
  refreshSeoPages,
  retryRefresh,
  loadPreviewImages,
  applyStatusEvent,
  applyCaptureEvent,
  appendOutputEvent,
  generation,
  resetRunIdentity,
  disposeCrawlRun,
} = useCrawlRun({
  activeCapture,
  activePageUrl,
  activeSizeId,
  captures,
  seoPages,
  seoLoading,
  outputBuffer,
  capturesRefreshError,
  seoRefreshError,
  previewUrls,
  thumbnailLoading,
  captureFilter,
  capturePage,
  searchInput,
  searchQuery,
  clearPreviewState: () => clearPreviewState(),
  status,
  starting,
  galleryView,
  activeTab,
  loading,
  isBusy,
  errorMessage,
  infoMessage,
  validationErrors,
  configuration,
  persistLocal,
  queueNativeSave,
  captureKey,
  isDisposed,
})

const {
  clearPreviewState,
  captureSiteUrl,
  isActiveCapture,
  revealActiveCapture,
  openPreview,
  openSeoCapture,
  navigatePreview,
  handlePreviewImageError,
  handlePreviewImageLoad,
  retryPreview,
  invalidatePreviewRequest,
  closePreview,
  onModalKeydown,
  previewEntries,
  selectedPreviewIndex,
  canShowPreviousPreview,
  canShowNextPreview,
  previewImageMode,
  previewMaximized,
  previewImageLoaded,
  selectedCapture,
  selectedPageTitle,
  selectedImageUrl,
  previewError,
  previewPending,
  previewModal,
  previewImage,
  displayedPreviewUrl,
  displayedActivePageUrl,
} = usePreview({
  activeCapture,
  activePageUrl,
  galleryView,
  visibleCaptures,
  visibleSeoPages,
  capturePageIndex,
  galleryWorkspace,
  capturePage,
  CAPTURE_PAGE_SIZE,
  activeSizeId,
  gallerySizes,
})

const {
  onHistoryModalKeydown,
  formatScanDate,
  refreshScanRuns,
  openHistory,
  closeHistory,
  chooseScanFolder,
  openHistoricalRun,
  disposeHistory,
  historyOpen,
  historyModal,
  historyRuns,
  historyRoot,
  historyLoading,
  historyError,
  historyOpeningPath,
} = useHistory({
  configuration,
  isBusy,
  resetRunIdentity,
  applyStatus,
  activeTab,
  outputBuffer,
  currentRunSnapshot,
  refreshCaptures,
  refreshSeoPages,
  isDisposed,
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

function captureKey(capture: CaptureItem): string {
  return capture.path || `${capture.filename}-${capture.sizeId || 'size'}`
}

function captureImage(capture: CaptureItem): string {
  return previewUrls[captureKey(capture)] || ''
}

function captureThumbnailLoading(capture: CaptureItem): boolean {
  return thumbnailLoading.has(captureKey(capture))
}

async function openSiteUrl(url: string): Promise<void> {
  try { await openExternalUrl(url) } catch (error) { errorMessage.value = String(error) }
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
  hydrateLocal()
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
    const logGeneration = generation()
    try {
      const initialLog = await getLog()
      if (!disposed && token === lifecycleToken && logGeneration === generation()) outputBuffer.replace(initialLog)
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
  completeHydration()
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

function onKeydown(event: KeyboardEvent): void {
  if (historyOpen.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      closeHistory()
    } else if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault()
    }
    return
  }
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

watch(galleryView, () => { if (activeCapture.value) void revealActiveCapture() })

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
  outputBuffer.dispose()
  disposed = true
  lifecycleToken += 1
  disposeCrawlRun()
  invalidatePreviewRequest()
  disposeHistory()
  window.removeEventListener('keydown', onKeydown)
  unlisteners.forEach((unlisten) => unlisten?.())
})

function isDisposed(): boolean { return disposed }

</script>

<template>
  <main class="app-shell" :class="{ 'is-busy': isBusy }">
    <SettingsSidebar
      @error="errorMessage = $event"
      @size-removed="(id) => { if (captureFilter === id) captureFilter = 'all' }"
      :sidebar-open="sidebarOpen"
      :configuration="configuration"
      :controls-disabled="controlsDisabled"
      :section-open="sectionOpen"
      :metadata-only="metadataOnly"
      :is-busy="isBusy"
      :can-start="canStart"
      :starting="starting"
      :retryable-run="retryableRun"
      :saving-configuration="savingConfiguration"
      :storage-error="storageError"
      :save-error="saveError"
      :engine-version="engineVersion"
      :begin-crawl="beginCrawl"
      :cancel-crawl="cancelCrawl"
      :set-sidebar-element="(element) => { sidebarElement = element as HTMLElement | null }"
      :set-target-url-input="(element) => { targetUrlInput = element as HTMLInputElement | null }"
    />
    <div
      v-if="sidebarOpen"
      class="sidebar-scrim"
      aria-hidden="true"
      @click="closeSidebar"
    ></div>
    <section class="workspace" aria-label="キャプチャワークスペース">
      <header class="run-header">
        <button
          class="sidebar-toggle"
          type="button"
          aria-label="設定を表示"
          :aria-expanded="sidebarOpen"
          @click="openSidebar($event.currentTarget)"
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="M2.5 4.25h11M2.5 8h11M2.5 11.75h11" />
          </svg>
        </button>
        <div class="phase-icon" :class="statusTone(status.phase)" aria-hidden="true">
          <span v-if="isBusy" class="spinner"></span>
          <AppIcon v-else-if="status.phase === 'succeeded'" name="check" />
          <AppIcon v-else-if="status.phase === 'failed'" name="alert" />
          <span v-else class="phase-ring"></span>
        </div>
        <div class="run-title">
          <h2>{{ statusTitle() }}</h2>
          <p>{{ statusSubtitle() }}</p>
        </div>
        <div class="run-actions">
          <button type="button" :disabled="!isTauri" @click="openHistory($event.currentTarget)">
            <AppIcon name="history" />
            <span>過去のスキャン</span>
          </button>
          <button type="button" :disabled="!reportPath || !isTauri" @click="openReport">
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M3 2.5h7l3 3v8H3zM10 2.5v3h3M5.5 8h5M5.5 10.5h5" />
            </svg>
            <span>HTMLレポート</span>
          </button>
          <button type="button" :disabled="!status.runId || !isTauri" @click="openOutput">
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M2.5 4.5h4l1.25 1.5h5.75v7.5h-11zM2.5 4.5v-1h4l1.25 1" />
            </svg>
            <span>保存先</span>
          </button>
        </div>
      </header>
      <div
        v-if="isBusy"
        class="progress-strip"
        role="progressbar"
        :aria-valuenow="runMetadataOnly ? undefined : currentProgress"
        :aria-valuemin="0"
        :aria-valuemax="100"
        :aria-label="progressLabel"
      >
        <span :style="{ transform: `scaleX(${currentProgress / 100})` }"></span>
      </div>
      <div v-if="isBusy" class="progress-label">{{ progressLabel }}<span v-if="status.currentSizeLabel"> · {{ status.currentSizeLabel }}</span></div>
      <div
        v-if="errorMessage || infoMessage || refreshError || storageError || saveError || validationErrors.length || (status.phase === 'failed' && status.message)"
        class="message-area"
        role="status"
      >
        <p v-if="errorMessage" class="message error">{{ errorMessage }}</p>
        <p v-else-if="status.phase === 'failed' && status.message" class="message error">{{ status.message }}</p>
        <p v-else-if="validationErrors.length && !isBusy" class="message error">{{ validationErrors[0] }}</p>
        <p v-if="infoMessage" class="message info">{{ infoMessage }}</p>
        <div v-if="refreshError" class="message error refresh-notice" role="alert">
          <span>{{ refreshError }}</span>
          <button type="button" @click="retryRefresh">再試行</button>
        </div>
      </div>
      <nav class="tabs" aria-label="表示切替">
        <button
          type="button"
          :class="{ active: activeTab === 'captures' }"
          @click="activeTab = 'captures'"
        >{{ runMetadataOnly ? 'メタ情報' : 'キャプチャ' }} <span v-if="runMetadataOnly ? seoPages.length : captures.length">{{ runMetadataOnly ? seoPages.length : captures.length }}</span></button>
        <button type="button" :class="{ active: activeTab === 'logs' }" @click="activeTab = 'logs'">ログ</button>
      </nav>
      <div v-if="activeTab === 'captures'" ref="galleryWorkspace" class="gallery-workspace">
        <div class="gallery-toolbar">
          <div class="filter-pills">
            <button
              v-for="option in filterOptions"
              :key="option.id"
              type="button"
              :class="{ active: captureFilter === option.id }"
              :aria-pressed="captureFilter === option.id"
              @click="captureFilter = option.id"
            >{{ option.label }} <span>{{ option.count }}</span></button>
          </div>
          <form class="gallery-search" role="search" @submit.prevent="applySearch">
            <label class="sr-only" for="gallery-search-input">キャプチャとページを検索</label>
            <input
              id="gallery-search-input"
              v-model="searchInput"
              type="search"
              placeholder="URL・タイトル・ファイル名を検索"
              autocomplete="off"
            />
            <button type="submit" aria-label="検索">
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <circle cx="7" cy="7" r="4.25" />
                <path d="m10.25 10.25 3 3" />
              </svg>
            </button>
            <button
              v-if="searchInput || searchQuery"
              class="search-clear"
              type="button"
              aria-label="検索を解除"
              @click="clearSearch"
            >
              <AppIcon name="close" />
            </button>
          </form>
          <div class="sort-tools">
            <label for="sort-order">並び替え:</label>
            <select id="sort-order" v-model="sortOrder" class="sort-select">
              <template v-if="galleryView === 'grid'">
                <option value="newest">取得日時（新しい順）</option>
                <option value="name">ファイル名</option>
                <option value="size">ファイルサイズ</option>
              </template>
              <template v-else>
                <option value="url">URL</option>
                <option value="title">タイトル</option>
                <option value="newest">取得日時（新しい順）</option>
              </template>
            </select>
            <div class="view-mode-toggle" role="group" aria-label="ギャラリー表示">
              <button
                class="view-toggle"
                :class="{ active: galleryView === 'grid' }"
                type="button"
                :aria-pressed="galleryView === 'grid'"
                aria-label="グリッド表示"
                @click="galleryView = 'grid'"
              >
                <AppIcon name="grid" />
                <span class="view-toggle-text">グリッド</span>
              </button>
              <button
                class="view-toggle"
                :class="{ active: galleryView === 'list' }"
                type="button"
                :aria-pressed="galleryView === 'list'"
                aria-label="行表示"
                @click="galleryView = 'list'"
              >
                <AppIcon name="list" />
                <span class="view-toggle-text">行</span>
              </button>
            </div>
          </div>
        </div>
        <div v-if="loading" class="empty-state">
          <span class="empty-icon" aria-hidden="true">
            <span class="spinner"></span>
          </span>
          <h3>準備しています</h3>
          <p>設定を読み込んでいます。</p>
        </div>
        <div v-else-if="galleryView === 'grid' && runMetadataOnly" class="empty-state">
          <span class="empty-icon">
            <AppIcon name="list" />
          </span>
          <h3>キャプチャなしで実行しています</h3>
          <p>取得したメタ情報は行表示で確認できます。</p>
          <button type="button" class="empty-action" @click="galleryView = 'list'">メタ情報を表示</button>
        </div>
        <div v-else-if="galleryView === 'grid' && visibleCaptures.length === 0" class="empty-state">
          <span class="empty-icon">
            <AppIcon name="image" />
          </span>
          <h3>{{ searchQuery ? '検索結果がありません' : isBusy ? 'キャプチャを待っています' : 'キャプチャはまだありません' }}</h3>
          <p v-if="searchQuery">URL・タイトル・ファイル名を確認してください。</p>
          <p v-else>{{ isBusy ? '撮影されたページから順に表示します。' : isTauri ? '左側で設定し、クロールを開始してください。' : 'ここではサンプルデータを表示します。実行はデスクトップアプリで行ってください。' }}</p>
          <button
            v-if="searchQuery"
            type="button"
            class="empty-action"
            @click="clearSearch"
          >検索を解除</button>
          <button
            v-else-if="isTauri && !isBusy"
            type="button"
            class="empty-action"
            @click="focusTargetUrl"
          >URLを入力する</button>
        </div>
        <div
          v-else-if="galleryView === 'list' && seoLoading && seoPages.length === 0"
          class="empty-state"
        >
          <span class="empty-icon" aria-hidden="true">
            <span class="spinner"></span>
          </span>
          <h3>SEOデータを読み込んでいます</h3>
          <p>レポートからページ情報を整理しています。</p>
        </div>
        <div v-else-if="galleryView === 'list' && visibleSeoPages.length === 0" class="empty-state">
          <span class="empty-icon">
            <AppIcon name="list" />
          </span>
          <h3>{{ searchQuery ? '検索結果がありません' : 'SEOデータは未取得です' }}</h3>
          <p>{{ searchQuery ? 'URL・タイトル・SEO項目を確認してください。' : 'クロール完了後にSiteOneのJSONレポートから表示します。' }}</p>
          <button
            v-if="searchQuery"
            type="button"
            class="empty-action"
            @click="clearSearch"
          >検索を解除</button>
          <button
            v-else-if="isTauri && !isBusy"
            type="button"
            class="empty-action"
            @click="focusTargetUrl"
          >URLを入力する</button>
        </div>
        <CaptureGrid
          v-else-if="galleryView === 'grid'"
          :paged-visible-captures="pagedVisibleCaptures"
          :is-active-capture="isActiveCapture"
          :open-preview="openPreview"
          :capture-site-url="captureSiteUrl"
          :capture-preview="capturePreview"
          :capture-image="captureImage"
          :capture-thumbnail-loading="captureThumbnailLoading"
          :open-capture="openCapture"
          :reveal-capture="revealCapture"
          :open-site-url="openSiteUrl"
        />
        <SeoResultsTable
          v-else
          :pages="visibleSeoPages"
          :sizes="gallerySizes"
          :active-page-url="displayedActivePageUrl"
          :active-size-id="activeSizeId"
          @open-url="openSiteUrl"
          @open-capture="openSeoCapture"
        />
        <footer class="gallery-footer">
          <template v-if="galleryView === 'grid'">
            <span>{{ visibleCaptures.length }}件のキャプチャ</span>
            <span v-if="enabledSizes.length"> · {{ enabledSizes.length }}サイズを順次処理</span>
            <nav v-if="capturePageCount > 1" class="pagination" aria-label="キャプチャページ">
              <button
                type="button"
                :disabled="capturePage <= 1"
                aria-label="前のページ"
                @click="setCapturePage(capturePage - 1)"
              >
                <AppIcon name="previous" />
              </button>
              <span>{{ capturePage }} / {{ capturePageCount }}</span>
              <button
                type="button"
                :disabled="capturePage >= capturePageCount"
                aria-label="次のページ"
                @click="setCapturePage(capturePage + 1)"
              >
                <AppIcon name="next" />
              </button>
            </nav>
          </template>
          <template v-else>{{ visibleSeoPages.length }}件のページ · SEO概要</template>
        </footer>
      </div>
      <div v-else class="log-workspace">
        <div class="log-toolbar">
          <span>実行ログ</span>
          <button
            type="button"
            :disabled="!logText"
            @click="clearLog().then(() => outputBuffer.clear())"
          >ログを消去</button>
        </div>
        <pre>{{ logText || 'クロールを開始するとログが表示されます。' }}</pre>
      </div>
    </section>
    <HistoryModal
      v-if="historyOpen"
      :is-busy="isBusy"
      :history-open="historyOpen"
      :history-root="historyRoot"
      :history-loading="historyLoading"
      :history-opening-path="historyOpeningPath"
      :history-error="historyError"
      :history-runs="historyRuns"
      :on-history-modal-keydown="onHistoryModalKeydown"
      :close-history="closeHistory"
      :choose-scan-folder="chooseScanFolder"
      :refresh-scan-runs="refreshScanRuns"
      :format-scan-date="formatScanDate"
      :open-historical-run="openHistoricalRun"
      :set-history-modal="(element) => { historyModal = element as HTMLElement | null }"
    />
    <PreviewModal
      v-if="selectedCapture"
      :open-capture="openCapture"
      :reveal-capture="revealCapture"
      :open-site-url="openSiteUrl"
      :selected-capture="selectedCapture"
      v-model:previewMaximized="previewMaximized"
      :selected-page-title="selectedPageTitle"
      :displayed-preview-url="displayedPreviewUrl"
      :selected-image-url="selectedImageUrl"
      :preview-pending="previewPending"
      :preview-error="previewError"
      :preview-image-loaded="previewImageLoaded"
      v-model:previewImageMode="previewImageMode"
      :preview-image="previewImage"
      :can-show-previous-preview="canShowPreviousPreview"
      :can-show-next-preview="canShowNextPreview"
      :selected-preview-index="selectedPreviewIndex"
      :preview-entries="previewEntries"
      :close-preview="closePreview"
      :on-modal-keydown="onModalKeydown"
      :handle-preview-image-load="handlePreviewImageLoad"
      :handle-preview-image-error="handlePreviewImageError"
      :retry-preview="retryPreview"
      :navigate-preview="navigatePreview"
      :set-preview-modal="(element) => { previewModal = element as HTMLElement | null }"
      :set-preview-image="(element) => { previewImage = element as HTMLImageElement | null }"
    />
  </main>
</template>

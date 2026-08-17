<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import {
  clearLog,
  getEngineVersion,
  getLog,
  getStatus,
  isTauri,
  listCaptures,
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
  type RunPhase,
} from './types'

const LOCAL_STORAGE_KEY = 'maho-crawl.configuration.v1'
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
const previewUrls = reactive<Record<string, string>>({})
const previewLoading = new Set<string>()
const selectedCapture = ref<CaptureItem | null>(null)
const selectedImageUrl = ref('')
const previewError = ref('')
const activeTab = ref<'captures' | 'logs'>('captures')
const captureFilter = ref('all')
const sortOrder = ref<CaptureSortOrder>('newest')
const logText = ref('')
const errorMessage = ref('')
const infoMessage = ref('')
const engineVersion = ref('確認中…')
const loading = ref(true)
const showAdvanced = ref(false)
const editingId = ref<string | null>(null)
const newSize = reactive({ label: '', width: 1280, height: 800 })
const unlisteners: Array<(() => void) | null> = []

const isBusy = computed(() => status.value.phase === 'running' || status.value.phase === 'cancelling')
const validationErrors = computed(() => validateConfiguration(configuration))
const canStart = computed(() => !isBusy.value && validationErrors.value.length === 0)
const enabledSizes = computed(() => configuration.captures.filter((capture) => capture.enabled))
const gallerySizes = computed(() => status.value.runCaptures.length ? status.value.runCaptures : configuration.captures)
const filterOptions = computed(() => [
  { id: 'all', label: 'すべて', count: captures.value.length },
  ...gallerySizes.value.map((capture) => ({
    id: capture.id,
    label: capture.label,
    count: captures.value.filter((item) => item.sizeId === capture.id).length,
  })),
])
const visibleCaptures = computed(() => {
  return filterAndSortCaptures(captures.value, captureFilter.value, sortOrder.value)
})
const currentProgress = computed(() => {
  if (!status.value.sizeTotal) return 0
  return Math.min(100, Math.round((Math.max(0, status.value.sizeIndex - (status.value.phase === 'running' ? 1 : 0)) / status.value.sizeTotal) * 100))
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
  if (status.value.phase === 'running') return status.value.currentSizeLabel ? `${status.value.currentSizeLabel}を撮影中` : 'クロールを準備しています'
  if (status.value.phase === 'cancelling') return '停止処理中'
  if (status.value.phase === 'succeeded') return 'クロール完了'
  if (status.value.phase === 'cancelled') return 'クロールを中止しました'
  if (status.value.phase === 'failed') return 'クロールを開始できません'
  return 'クロール設定を入力してください'
}

function statusSubtitle(): string {
  if (status.value.currentPlan?.root) return status.value.currentPlan.root
  if (status.value.runId) return status.value.runId
  return 'URL・UA・画面サイズを指定してキャプチャできます'
}

function persistLocal(): void {
  localStorage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(configuration))
}

async function persist(): Promise<void> {
  persistLocal()
  if (!isTauri || isBusy.value) return
  try {
    await saveConfiguration(cloneConfiguration(configuration))
  } catch {
    // Local storage remains the fallback when a desktop config directory is unavailable.
  }
}

function normalizeLoaded(value: Partial<CrawlConfiguration>): void {
  const merged = { ...cloneConfiguration(DEFAULT_CONFIGURATION), ...value }
  const nextCaptures = Array.isArray(value.captures) && value.captures.length > 0 ? value.captures : DEFAULT_CONFIGURATION.captures
  Object.assign(configuration, {
    ...merged,
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

function addSize(): void {
  const label = newSize.label.trim()
  const width = Number(newSize.width)
  const height = Number(newSize.height)
  if (!label || !Number.isInteger(width) || !Number.isInteger(height)) {
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
  if (configuration.captures.length <= 1) {
    errorMessage.value = 'キャプチャサイズは最低1件必要です。'
    return
  }
  configuration.captures = configuration.captures.filter((capture) => capture.id !== id)
  if (captureFilter.value === id) captureFilter.value = 'all'
}

function toggleEdit(id: string): void {
  editingId.value = editingId.value === id ? null : id
}

function ensureInteger(viewport: CaptureViewport, key: 'width' | 'height'): void {
  viewport[key] = Math.round(Number(viewport[key]) || 0)
}

function displayedLabel(viewport: CaptureViewport): string {
  return viewport.label || DEFAULT_CONFIGURATION.captures.find((item) => item.id === viewport.id)?.label || viewport.id
}

function displayedDimension(viewport: CaptureViewport, key: 'width' | 'height'): number {
  if (Number.isFinite(viewport[key]) && viewport[key] > 0) return viewport[key]
  const fallback = DEFAULT_CONFIGURATION.captures.find((item) => item.id === viewport.id)
  return fallback?.[key] || (key === 'width' ? 1280 : 800)
}

function updateLabel(viewport: CaptureViewport, event: Event): void {
  viewport.label = (event.target as HTMLInputElement).value
}

function updateDimension(viewport: CaptureViewport, key: 'width' | 'height', event: Event): void {
  viewport[key] = Number((event.target as HTMLInputElement).value)
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

async function beginCrawl(): Promise<void> {
  errorMessage.value = ''
  infoMessage.value = ''
  if (validationErrors.value.length) {
    errorMessage.value = validationErrors.value[0]
    return
  }
  try {
    if (!isTauri) {
      persistLocal()
      status.value = {
        phase: 'running', runId: `demo-${Date.now()}`, sizeIndex: 1, sizeTotal: enabledSizes.value.length,
        currentSizeId: enabledSizes.value[0]?.id ?? null, currentSizeLabel: enabledSizes.value[0]?.label ?? null,
        currentPlan: null, plans: [], runCaptures: enabledSizes.value,
        message: 'ブラウザプレビューでは実行をシミュレートしています',
      }
      infoMessage.value = 'ブラウザプレビューでは sidecar を実行しません。'
      return
    }
    await validateConfigurationRust(cloneConfiguration(configuration))
    persistLocal()
    const response = await startCrawl(cloneConfiguration(configuration))
    status.value = { ...status.value, phase: 'running', runId: response.runId, sizeTotal: response.totalSizes }
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function cancelCrawl(): Promise<void> {
  try {
    if (isTauri) await stopCrawl()
    else status.value = { ...status.value, phase: 'cancelled', message: '停止しました' }
  } catch (error) {
    errorMessage.value = String(error)
  }
}

async function refreshCaptures(): Promise<void> {
  if (!isTauri || !status.value.currentPlan?.root) return
  try {
    captures.value = await listCaptures(status.value.currentPlan.root, cloneConfiguration(configuration))
    await loadPreviewImages(captures.value)
  } catch {
    // A run can emit its status before the first output directory is visible.
  }
}

function captureKey(capture: CaptureItem): string {
  return capture.path || `${capture.filename}-${capture.sizeId || 'size'}`
}

async function loadPreviewImages(items: CaptureItem[]): Promise<void> {
  if (!isTauri) return
  for (const capture of items) {
    const key = captureKey(capture)
    if (previewUrls[key] || previewLoading.has(key)) continue
    previewLoading.add(key)
    try {
      const image = await readCaptureThumbnail(capture.path)
      previewUrls[key] = `data:${image.mimeType};base64,${image.dataBase64}`
    } catch {
      // Keep the neutral placeholder for a deleted or still-being-written image.
    } finally {
      previewLoading.delete(key)
    }
  }
}

function captureImage(capture: CaptureItem): string {
  return previewUrls[captureKey(capture)] || ''
}

async function openPreview(capture: CaptureItem): Promise<void> {
  selectedCapture.value = capture
  previewError.value = ''
  selectedImageUrl.value = ''
  if (!isTauri) return
  try {
    const image = await readCapture(capture.path)
    selectedImageUrl.value = `data:${image.mimeType};base64,${image.dataBase64}`
  } catch (error) {
    previewError.value = String(error)
  }
}

function closePreview(): void {
  selectedCapture.value = null
  selectedImageUrl.value = ''
  previewError.value = ''
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
  const saved = localStorage.getItem(LOCAL_STORAGE_KEY)
  if (saved) {
    try { normalizeLoaded(JSON.parse(saved) as Partial<CrawlConfiguration>) } catch { /* ignore malformed local storage */ }
  }
  if (isTauri) {
    try { normalizeLoaded(await loadConfiguration()) } catch { /* local storage fallback */ }
    try { engineVersion.value = await getEngineVersion() } catch { engineVersion.value = '利用不可' }
    try { status.value = await getStatus() } catch { /* idle fallback */ }
    try { logText.value = await getLog() } catch { /* empty fallback */ }
  } else {
    engineVersion.value = 'プレビュー'
    captures.value = demoCaptures()
  }
  ensureConfigurationDefaults()
  loading.value = false
}

function demoCaptures(): CaptureItem[] {
  return configuration.captures.flatMap((viewport, viewportIndex) => [0, 1].map((index) => ({
    path: '', filename: `${index === 0 ? 'home' : 'about'}-${viewport.id}.png`, bytes: 430000 + viewportIndex * 210000 + index * 38000,
    modifiedAt: Date.now() - (viewportIndex * 2 + index) * 60_000, sizeId: viewport.id, sizeLabel: viewport.label,
    width: viewport.width, height: viewport.height,
  })))
}

function onKeydown(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && canStart.value) {
    event.preventDefault()
    void beginCrawl()
  }
}

watch(configuration, () => { void persist() }, { deep: true })

onMounted(async () => {
  window.addEventListener('keydown', onKeydown)
  await loadInitialState()
  if (isTauri) {
    unlisteners.push(await subscribeStatus((next) => { status.value = next; void refreshCaptures() }))
    unlisteners.push(await subscribeOutput((text) => { logText.value += text; if (logText.value.length > 400_000) logText.value = logText.value.slice(-400_000) }))
    unlisteners.push(await subscribeCaptures((next) => { captures.value = next; void loadPreviewImages(next) }))
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  unlisteners.forEach((unlisten) => unlisten?.())
})
</script>

<template>
  <main class="app-shell" :class="{ 'is-busy': isBusy }">
    <aside class="settings-sidebar" aria-label="クロール設定">
      <div class="settings-scroll">
        <section class="setting-section">
          <div class="section-heading"><h2>クロール対象</h2><span>⌃</span></div>
          <label class="field-label" for="target-url">URL</label>
          <input id="target-url" v-model="configuration.targetUrl" class="text-input" type="url" placeholder="https://example.com" :disabled="isBusy" />
          <label class="switch-row">
            <input v-model="configuration.singlePage" type="checkbox" :disabled="isBusy" />
            <span class="switch" aria-hidden="true"></span><span>このページのみ</span>
          </label>
          <label class="field-label" for="user-agent">ユーザーエージェント（カスタム）</label>
          <input id="user-agent" v-model="configuration.userAgent" class="text-input" type="text" placeholder="空欄ならDesktop" :disabled="isBusy" />
        </section>

        <section class="setting-section sizes-section">
          <div class="section-heading"><h2>キャプチャサイズ</h2><span>⌃</span></div>
          <p class="section-help">有効なサイズを順番に1回の実行で処理します。</p>
          <div v-for="viewport in configuration.captures" :key="viewport.id" class="size-row" :class="{ editing: editingId === viewport.id }">
            <label class="checkbox-label" :title="`${viewport.label}を有効化`">
              <input v-model="viewport.enabled" type="checkbox" :disabled="isBusy" :aria-label="`${viewport.label}を有効化`" />
              <span class="checkmark" aria-hidden="true">✓</span>
            </label>
            <div class="size-fields">
              <input :value="displayedLabel(viewport)" @input="updateLabel(viewport, $event)" class="size-label-input" :disabled="isBusy" :aria-label="`${displayedLabel(viewport)}の名前`" />
              <div class="dimension-fields">
                <input :value="displayedDimension(viewport, 'width')" @input="updateDimension(viewport, 'width', $event)" @change="ensureInteger(viewport, 'width')" class="dimension-input" type="number" min="320" max="8192" :disabled="isBusy" :aria-label="`${displayedLabel(viewport)}の幅`" />
                <span>×</span>
                <input :value="displayedDimension(viewport, 'height')" @input="updateDimension(viewport, 'height', $event)" @change="ensureInteger(viewport, 'height')" class="dimension-input" type="number" min="320" max="8192" :disabled="isBusy" :aria-label="`${displayedLabel(viewport)}の高さ`" />
              </div>
            </div>
            <button class="icon-button" type="button" :disabled="isBusy" :aria-label="`${viewport.label}を編集`" @click="toggleEdit(viewport.id)">{{ editingId === viewport.id ? '✓' : '⌕' }}</button>
            <button class="icon-button delete" type="button" :disabled="isBusy" :aria-label="`${viewport.label}を削除`" @click="removeSize(viewport.id)">×</button>
          </div>
          <div class="add-size-row">
            <input v-model="newSize.label" class="size-label-input" type="text" placeholder="新しいサイズ" :disabled="isBusy" @keydown.enter="addSize" />
            <input v-model.number="newSize.width" class="dimension-input" type="number" min="320" max="8192" aria-label="新しいサイズの幅" :disabled="isBusy" />
            <span>×</span>
            <input v-model.number="newSize.height" class="dimension-input" type="number" min="320" max="8192" aria-label="新しいサイズの高さ" :disabled="isBusy" />
            <button class="add-button" type="button" :disabled="isBusy" @click="addSize">＋ 追加</button>
          </div>
        </section>

        <section class="setting-section">
          <div class="section-heading"><h2>撮影設定</h2><span>⌃</span></div>
          <div class="field-grid">
            <label class="field-label">撮影モード
              <select v-model="configuration.screenshotMode" class="select-input" :disabled="isBusy"><option value="viewport">表示領域</option><option value="full-page">ページ全体</option></select>
            </label>
            <label class="field-label">出力フォーマット
              <select v-model="configuration.screenshotFormat" class="select-input" :disabled="isBusy"><option value="png">PNG</option><option value="jpg">JPG</option><option value="webp">WebP</option></select>
            </label>
          </div>
          <label class="switch-row">
            <input v-model="configuration.hideCookieBanner" type="checkbox" :disabled="isBusy" />
            <span class="switch" aria-hidden="true"></span><span>Cookieバナー非表示（対応サイトのみ）</span>
          </label>
        </section>

        <section class="setting-section">
          <button class="section-heading collapsible" type="button" @click="showAdvanced = !showAdvanced"><h2>クロール設定</h2><span>{{ showAdvanced ? '⌃' : '⌄' }}</span></button>
          <div v-show="showAdvanced" class="advanced-fields">
            <label class="field-label">最大深度<input v-model.number="configuration.maxDepth" class="number-input" type="number" min="0" max="20" :disabled="isBusy || configuration.singlePage" /></label>
            <label class="field-label">同時ワーカー数<input v-model.number="configuration.workers" class="number-input" type="number" min="1" max="16" :disabled="isBusy" /></label>
            <label class="field-label">ブラウザワーカー数<input v-model.number="configuration.browserWorkers" class="number-input" type="number" min="1" max="8" :disabled="isBusy" /></label>
            <label class="field-label">最大リクエスト / 秒<input v-model.number="configuration.maxRequestsPerSecond" class="number-input" type="number" min="1" max="100" :disabled="isBusy" /></label>
          </div>
        </section>

        <section class="setting-section">
          <div class="section-heading"><h2>ブラウザ</h2><span>⌃</span></div>
          <div class="field-grid">
            <label class="field-label">待機条件<select v-model="configuration.browserWait" class="select-input" :disabled="isBusy"><option value="networkidle">通信完了まで</option><option value="load">loadイベントまで</option><option value="domcontentloaded">DOM構築まで</option></select></label>
            <label class="field-label">タイムアウト（秒）<input v-model.number="configuration.browserTimeout" class="number-input" type="number" min="5" max="300" :disabled="isBusy" /></label>
          </div>
          <label class="field-label">ブラウザのパス<input v-model="configuration.browserPath" class="text-input" type="text" placeholder="自動検出" :disabled="isBusy" /></label>
          <label class="switch-row"><input v-model="configuration.autoDownloadBrowser" type="checkbox" :disabled="isBusy" /><span class="switch" aria-hidden="true"></span><span>見つからない場合に自動取得</span></label>
        </section>

        <section class="setting-section output-section">
          <div class="section-heading"><h2>保存先</h2><span>⌃</span></div>
          <div class="output-picker"><input v-model="configuration.outputRoot" class="text-input" type="text" :disabled="isBusy" /><button type="button" :disabled="isBusy || !isTauri" @click="chooseOutputFolder">参照…</button></div>
          <p class="path-note">実行ごとに日時フォルダ、サイズごとに専用フォルダを作成します。</p>
        </section>
      </div>

      <div class="sidebar-footer">
        <button v-if="isBusy" class="primary-button stop-button" type="button" @click="cancelCrawl"><span>■</span> クロールを中止</button>
        <button v-else class="primary-button" type="button" :disabled="!canStart" @click="beginCrawl"><span>▶</span> クロールを開始</button>
        <div class="engine-line"><span class="engine-dot" :class="{ online: engineVersion !== '利用不可' }"></span> Engine: {{ engineVersion }}</div>
        <div class="license-line">SiteOne Crawler (MIT) · 非公式ラッパー</div>
      </div>
    </aside>

    <section class="workspace" aria-label="キャプチャワークスペース">
      <header class="run-header">
        <div class="phase-icon" :class="statusTone(status.phase)"><span v-if="isBusy" class="spinner"></span><span v-else>{{ status.phase === 'succeeded' ? '✓' : status.phase === 'failed' ? '!' : '◌' }}</span></div>
        <div class="run-title"><h2>{{ statusTitle() }}</h2><p>{{ statusSubtitle() }}</p></div>
        <div class="run-actions"><button type="button" :disabled="!reportPath || !isTauri" @click="openReport">▧ <span>HTMLレポート</span></button><button type="button" :disabled="!status.runId || !isTauri" @click="openOutput">▱ <span>保存先</span></button><button class="more-button" type="button" aria-label="その他">⋮</button></div>
      </header>
      <div v-if="isBusy" class="progress-strip"><span :style="{ width: `${currentProgress}%` }"></span></div>
      <div v-if="errorMessage || infoMessage || validationErrors.length" class="message-area" role="status">
        <p v-if="errorMessage" class="message error">{{ errorMessage }}</p>
        <p v-else-if="validationErrors.length && !isBusy" class="message error">{{ validationErrors[0] }}</p>
        <p v-if="infoMessage" class="message info">{{ infoMessage }}</p>
      </div>

      <nav class="tabs" aria-label="表示切替"><button type="button" :class="{ active: activeTab === 'captures' }" @click="activeTab = 'captures'">キャプチャ <span v-if="captures.length">{{ captures.length }}</span></button><button type="button" :class="{ active: activeTab === 'logs' }" @click="activeTab = 'logs'">ログ</button></nav>

      <div v-if="activeTab === 'captures'" class="gallery-workspace">
        <div class="gallery-toolbar">
          <div class="filter-pills"><button v-for="option in filterOptions" :key="option.id" type="button" :class="{ active: captureFilter === option.id }" @click="captureFilter = option.id">{{ option.label }} <span>{{ option.count }}</span></button></div>
          <div class="sort-tools"><label for="sort-order">並び替え:</label><select id="sort-order" v-model="sortOrder" class="sort-select"><option value="newest">取得日時（新しい順）</option><option value="name">ファイル名</option><option value="size">ファイルサイズ</option></select><button class="view-toggle active" type="button" aria-label="グリッド表示">▦</button></div>
        </div>
        <div v-if="loading" class="empty-state"><span class="empty-icon">◌</span><h3>準備しています</h3><p>設定を読み込んでいます。</p></div>
        <div v-else-if="visibleCaptures.length === 0" class="empty-state"><span class="empty-icon">▧</span><h3>{{ isBusy ? 'キャプチャを待っています' : 'キャプチャはまだありません' }}</h3><p>{{ isBusy ? '撮影されたページから順に表示します。' : '左側で設定し、クロールを開始してください。' }}</p></div>
        <div v-else class="capture-grid">
          <article v-for="capture in visibleCaptures" :key="capture.path || capture.filename" class="capture-card" tabindex="0" @click="openPreview(capture)" @dblclick="openCapture(capture)" @keydown.enter="openPreview(capture)">
            <div class="capture-thumb" :style="{ background: capturePreview(capture) }"><img v-if="captureImage(capture)" :src="captureImage(capture)" :alt="`${capture.filename}のプレビュー`" /><div v-else class="mock-page"><span></span><i></i><b></b><em></em></div></div>
            <div class="capture-meta"><div class="capture-file"><span class="device-icon">{{ capture.width && capture.width > 1000 ? '▱' : '▯' }}</span><span class="truncate">{{ capture.filename }}</span></div><div class="capture-tags"><span class="size-tag" :class="capture.sizeId">{{ capture.sizeLabel || 'サイズ' }}</span><span>{{ capture.width }} × {{ capture.height }}</span></div><div class="capture-bottom"><span>{{ formatBytes(capture.bytes) }}</span><span class="capture-actions"><button type="button" @click.stop="openCapture(capture)" :disabled="!isTauri" aria-label="元画像を開く">↗</button><button type="button" @click.stop="revealCapture(capture)" :disabled="!isTauri" aria-label="Finderで表示">⌕</button></span></div></div>
          </article>
        </div>
        <footer class="gallery-footer">{{ visibleCaptures.length }}件のキャプチャ<span v-if="enabledSizes.length"> · {{ enabledSizes.length }}サイズを順次処理</span></footer>
      </div>
      <div v-else class="log-workspace"><div class="log-toolbar"><span>実行ログ</span><button type="button" :disabled="!logText" @click="clearLog().then(() => { logText = '' })">ログを消去</button></div><pre>{{ logText || 'クロールを開始するとログが表示されます。' }}</pre></div>
    </section>
    <div v-if="selectedCapture" class="preview-backdrop" role="presentation" @click.self="closePreview">
      <section class="preview-modal" role="dialog" aria-modal="true" :aria-label="`${selectedCapture.filename}のプレビュー`">
        <header><div><h2>{{ selectedCapture.filename }}</h2><p>{{ selectedCapture.sizeLabel }} · {{ selectedCapture.width }} × {{ selectedCapture.height }} · {{ formatBytes(selectedCapture.bytes) }}</p></div><button type="button" aria-label="プレビューを閉じる" @click="closePreview">×</button></header>
        <div class="preview-image-wrap"><img v-if="selectedImageUrl" :src="selectedImageUrl" :alt="`${selectedCapture.filename}のプレビュー`" /><div v-else-if="previewError" class="preview-error"><strong>プレビューを読み込めませんでした</strong><span>{{ previewError }}</span><small>元画像を開くか、Finderで表示してください。</small></div><div v-else class="mock-page large"><span></span><i></i><b></b><em></em></div></div>
        <footer><button type="button" :disabled="!isTauri" @click="openCapture(selectedCapture)">元画像を開く</button><button type="button" :disabled="!isTauri" @click="revealCapture(selectedCapture)">Finderで表示</button></footer>
      </section>
    </div>
  </main>
</template>

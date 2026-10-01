import { type Ref, type ComputedRef } from 'vue'
import { isTauri, listCaptures, listSeoPages, readCaptureThumbnail, startCrawl, stopCrawl, validateConfigurationRust } from '../bridge'
import { cloneConfiguration, sanitizeConfigurationForStorage, type CaptureItem, type CaptureViewport, type CrawlConfiguration, type CrawlStatus, type GalleryView, type RunPhase, type SeoPageItem } from '../types'
import { useOutputBuffer } from '../composables/useOutputBuffer'

import type { RunSnapshot } from './contracts'

export interface UseCrawlRunContext {
  activeCapture: Ref<CaptureItem | null>
  activePageUrl: Ref<string>
  activeSizeId: Ref<string | null>
  captures: Ref<CaptureItem[]>
  seoPages: Ref<SeoPageItem[]>
  seoLoading: Ref<boolean>
  outputBuffer: ReturnType<typeof useOutputBuffer>
  capturesRefreshError: Ref<string>
  seoRefreshError: Ref<string>
  previewUrls: Record<string, string>
  thumbnailLoading: Set<string>
  captureFilter: Ref<string>
  capturePage: Ref<number>
  searchInput: Ref<string>
  searchQuery: Ref<string>
  clearPreviewState: () => void
  status: Ref<CrawlStatus>
  starting: Ref<boolean>
  galleryView: Ref<GalleryView>
  activeTab: Ref<'captures' | 'logs'>
  loading: Ref<boolean>
  isBusy: ComputedRef<boolean>
  errorMessage: Ref<string>
  infoMessage: Ref<string>
  validationErrors: ComputedRef<string[]>
  configuration: CrawlConfiguration
  persistLocal: () => boolean
  queueNativeSave: (snapshot: CrawlConfiguration) => void
  captureKey: (capture: CaptureItem) => string
  isDisposed: () => boolean
}

export function useCrawlRun(context: UseCrawlRunContext) {
  const {
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
    clearPreviewState,
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
  } = context

  let activeRunId: string | null = null

  let activeRunRoot: string | null = null

  let runGeneration = 0

  let startAttemptToken = 0

  let capturesRefreshToken = 0

  let seoRefreshToken = 0

  function isTerminalPhase(phase: RunPhase): boolean {
    return phase === 'succeeded' || phase === 'cancelled' || phase === 'failed'
  }

  function resetRunArtifacts(): void {
    activeCapture.value = null
    activePageUrl.value = ''
    activeSizeId.value = null
    capturesRefreshToken += 1
    seoRefreshToken += 1
    captures.value = []
    seoPages.value = []
    seoLoading.value = false
    outputBuffer.clear()
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
    if (isDisposed()) return false
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
    return !isDisposed()
      && snapshot.runId === activeRunId
      && snapshot.root === activeRunRoot
      && snapshot.generation === runGeneration
      && status.value.runId === snapshot.runId
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
      if (isDisposed() || token !== startAttemptToken) return
      persistLocal()
      queueNativeSave(sanitizeConfigurationForStorage(snapshot))
      const response = await startCrawl(snapshot)
      if (isDisposed() || token !== startAttemptToken) return
      applyAcceptedStart(response, runCaptures)
    } catch (error) {
      if (!isDisposed() && token === startAttemptToken) errorMessage.value = String(error)
    } finally {
      if (!isDisposed() && token === startAttemptToken) starting.value = false
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
      const nextCaptures = await listCaptures(snapshot.root)
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
    if (isDisposed() || !activeRunId || status.value.phase === 'idle') return
    outputBuffer.append(text)
  }

  function generation(): number { return runGeneration }

  function resetRunIdentity(): void { activeRunId = null; activeRunRoot = null }

  function disposeCrawlRun(): void { startAttemptToken += 1 }

  return {
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
  }
}

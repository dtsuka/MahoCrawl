import { nextTick, ref, type Ref, type ComputedRef } from 'vue'
import { isTauri, listScanRuns, loadScanRun, selectScanFolder } from '../bridge'
import { type CrawlConfiguration, type CrawlStatus, type ScanRunSummary } from '../types'
import { useOutputBuffer } from '../composables/useOutputBuffer'

import type { RunSnapshot } from './contracts'

export interface UseHistoryContext {
  configuration: CrawlConfiguration
  isBusy: ComputedRef<boolean>
  resetRunIdentity: () => void
  applyStatus: (next: CrawlStatus) => boolean
  activeTab: Ref<'captures' | 'logs'>
  outputBuffer: ReturnType<typeof useOutputBuffer>
  currentRunSnapshot: () => RunSnapshot | null
  refreshCaptures: (snapshot?: RunSnapshot | null) => Promise<void>
  refreshSeoPages: (snapshot?: RunSnapshot | null) => Promise<void>
  isDisposed: () => boolean
}

export function useHistory(context: UseHistoryContext) {
  const {
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
  } = context

  const historyOpen = ref(false)

  const historyModal = ref<HTMLElement | null>(null)

  const historyOpener = ref<HTMLElement | null>(null)

  const historyRuns = ref<ScanRunSummary[]>([])

  const historyRoot = ref('')

  const historyLoading = ref(false)

  const historyError = ref('')

  const historyOpeningPath = ref('')

  let historyRequestToken = 0

  function onHistoryModalKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      closeHistory()
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
    const modal = historyModal.value
    if (!modal) return
    const focusable = Array.from(modal.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ))
    if (!focusable.length) {
      modal.focus()
      return
    }
    const currentIndex = focusable.indexOf(document.activeElement as HTMLElement)
    const nextIndex = currentIndex < 0
      ? (event.shiftKey ? focusable.length - 1 : 0)
      : event.shiftKey
        ? (currentIndex - 1 + focusable.length) % focusable.length
        : (currentIndex + 1) % focusable.length
    focusable[nextIndex].focus()
  }

  function formatScanDate(timestamp: number): string {
    if (!timestamp) return '日時不明'
    return new Intl.DateTimeFormat('ja-JP', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(timestamp * 1000))
  }

  async function refreshScanRuns(root = historyRoot.value): Promise<void> {
    if (!isTauri || !root) return
    const token = ++historyRequestToken
    historyLoading.value = true
    historyError.value = ''
    try {
      const runs = await listScanRuns(root)
      if (isDisposed() || token !== historyRequestToken || !historyOpen.value) return
      historyRuns.value = runs
      historyRoot.value = root
    } catch (error) {
      if (!isDisposed() && token === historyRequestToken && historyOpen.value) {
        historyRuns.value = []
        historyError.value = `スキャン一覧を読み込めません。${String(error)}`
      }
    } finally {
      if (!isDisposed() && token === historyRequestToken) historyLoading.value = false
    }
  }

  async function openHistory(opener?: EventTarget | null): Promise<void> {
    if (!isTauri || historyOpen.value) return
    historyOpener.value = opener as HTMLElement | null
    historyRoot.value = configuration.outputRoot
    historyRuns.value = []
    historyError.value = ''
    historyOpen.value = true
    await nextTick()
    historyModal.value?.focus()
    await refreshScanRuns(configuration.outputRoot)
  }

  function closeHistory(): void {
    if (!historyOpen.value) return
    historyRequestToken += 1
    const opener = historyOpener.value
    historyOpen.value = false
    historyLoading.value = false
    historyOpeningPath.value = ''
    historyOpener.value = null
    void nextTick(() => opener?.isConnected && opener.focus())
  }

  async function chooseScanFolder(): Promise<void> {
    if (!isTauri || historyLoading.value || historyOpeningPath.value) return
    try {
      const selected = await selectScanFolder()
      if (!selected || !historyOpen.value) return
      historyRoot.value = selected
      await refreshScanRuns(selected)
    } catch (error) {
      if (historyOpen.value) historyError.value = `フォルダを選択できません。${String(error)}`
    }
  }

  async function openHistoricalRun(run: ScanRunSummary): Promise<void> {
    if (!isTauri || isBusy.value || historyOpeningPath.value) return
    historyOpeningPath.value = run.path
    historyError.value = ''
    try {
      const nextStatus = await loadScanRun(run.path)
      if (isDisposed() || !historyOpen.value) return
      resetRunIdentity()
      if (!applyStatus(nextStatus)) throw new Error('スキャンの表示状態を更新できません。')
      activeTab.value = 'captures'
      outputBuffer.clear()
      closeHistory()
      const snapshot = currentRunSnapshot()
      if (snapshot) await Promise.all([refreshCaptures(snapshot), refreshSeoPages(snapshot)])
    } catch (error) {
      if (!isDisposed() && historyOpen.value) historyError.value = `スキャンを開けません。${String(error)}`
    } finally {
      historyOpeningPath.value = ''
    }
  }

  function disposeHistory(): void { historyRequestToken += 1 }

  return {
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
  }
}

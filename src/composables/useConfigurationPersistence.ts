import { ref, watch, type Ref, type ComputedRef } from 'vue'
import { isTauri, saveConfiguration } from '../bridge'
import { cloneConfiguration, sanitizeConfigurationForStorage, DEFAULT_CONFIGURATION, validateConfiguration, type CrawlConfiguration } from '../types'

export interface UseConfigurationPersistenceContext {
  configuration: CrawlConfiguration
  loading: Ref<boolean>
  configurationReady: Ref<boolean>
  isBusy: ComputedRef<boolean>
}

export function useConfigurationPersistence(context: UseConfigurationPersistenceContext) {
  const { configuration, loading, configurationReady, isBusy } = context

  const LOCAL_STORAGE_KEY = 'maho-crawl.configuration.v1'

  const storageError = ref('')

  const saveError = ref('')

  const savingConfiguration = ref(false)

  let initialHydration = true

  let hydrationWatchPending = false

  let saveRevision = 0

  let pendingSave: { revision: number; configuration: CrawlConfiguration } | null = null

  let saveQueue: Promise<void> | null = null

  function hydrateLocal(): void {
    let saved: string | null = null
    try {
      saved = localStorage.getItem(LOCAL_STORAGE_KEY)
    } catch (error) {
      storageError.value = `保存済み設定を読み込めません。${String(error)}`
    }
    if (saved) {
      try { normalizeLoaded(JSON.parse(saved) as Partial<CrawlConfiguration>) } catch { /* ignore malformed local storage */ }
    }
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

  function completeHydration(): void { hydrationWatchPending = true; configurationReady.value = true; loading.value = false; initialHydration = false }

  watch(configuration, () => {
    if (initialHydration) return
    if (hydrationWatchPending) {
      hydrationWatchPending = false
      return
    }
    void persist()
  }, { deep: true })

  return {
    hydrateLocal,
    persistLocal,
    queueNativeSave,
    normalizeLoaded,
    ensureConfigurationDefaults,
    completeHydration,
    storageError,
    saveError,
    savingConfiguration,
  }
}

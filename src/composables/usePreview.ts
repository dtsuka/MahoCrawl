import { computed, nextTick, reactive, ref, watch, type Ref, type ComputedRef } from 'vue'
import { isTauri, readCapture } from '../bridge'
import { type CaptureItem, type CaptureViewport, type GalleryView, type SeoPageItem } from '../types'
import { captureIdentity, pageForCapture, pageSizeOptions } from '../gallery'
import { externalUrl } from '../urls'

import type { PreviewImageMode, PreviewRequest, PreviewContext, PreviewEntry } from './contracts'

export interface UsePreviewContext {
  activeCapture: Ref<CaptureItem | null>
  activePageUrl: Ref<string>
  galleryView: Ref<GalleryView>
  visibleCaptures: ComputedRef<CaptureItem[]>
  visibleSeoPages: ComputedRef<SeoPageItem[]>
  capturePageIndex: ComputedRef<Map<string, SeoPageItem[]>>
  galleryWorkspace: Ref<HTMLElement | null>
  capturePage: Ref<number>
  CAPTURE_PAGE_SIZE: number
  activeSizeId: Ref<string | null>
  gallerySizes: ComputedRef<CaptureViewport[]>
}

export function usePreview(context: UsePreviewContext) {
  const {
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
  } = context

  const PREVIEW_IMAGE_MODE_STORAGE_KEY = 'maho-crawl.preview-image-mode.v1'

  const selectedCapture = ref<CaptureItem | null>(null)

  const selectedPageTitle = ref('')

  const selectedPageUrl = ref('')

  const displayedPreviewUrl = computed(() => selectedPageUrl.value
    || (selectedCapture.value ? captureContext(selectedCapture.value).pageUrl : '') || '')

  const displayedActivePageUrl = computed(() => activeCapture.value
    ? activePageUrl.value || captureContext(activeCapture.value).pageUrl : undefined)

  const selectedImageUrl = ref('')

  const previewError = ref('')

  const previewPending = ref(false)

  const previewModal = ref<HTMLElement | null>(null)

  const previewOpener = ref<HTMLElement | null>(null)

  let previewRequestToken = 0

  let activePreviewRequest: PreviewRequest | null = null

  const previewImage = ref<HTMLImageElement | null>(null)

  function readPreviewImageModePreference(): PreviewImageMode {
    try {
      const saved = localStorage.getItem(PREVIEW_IMAGE_MODE_STORAGE_KEY)
      return saved === 'actual' || saved === 'width' ? saved : 'fit'
    } catch {
      return 'fit'
    }
  }

  const previewImageMode = ref<PreviewImageMode>(readPreviewImageModePreference())

  const previewMaximized = ref(false)

  const previewImageLoaded = ref(false)

  const previewEntries = computed<PreviewEntry[]>(() => {
    if (galleryView.value === 'grid') {
      return visibleCaptures.value.map((capture) => ({ capture, context: captureContext(capture) }))
    }
    return visibleSeoPages.value.flatMap((page) => page.sizeIds.map((sizeId) => seoPreviewEntry(page, sizeId)))
  })

  const selectedPreviewIndex = computed(() => {
    const request = activePreviewRequest
    // selectedCapture is intentionally read here so this computed value updates
    // when the non-reactive request guard moves to another image.
    if (!request || !selectedCapture.value) return -1
    return previewEntries.value.findIndex((entry) => previewEntryMatches(entry, request))
  })

  const canShowPreviousPreview = computed(() => selectedPreviewIndex.value > 0)

  const canShowNextPreview = computed(() => (
    selectedPreviewIndex.value >= 0 && selectedPreviewIndex.value < previewEntries.value.length - 1
  ))

  function clearPreviewState(): void {
    invalidatePreviewRequest()
    selectedCapture.value = null
    selectedPageTitle.value = ''
    selectedPageUrl.value = ''
    selectedImageUrl.value = ''
    previewError.value = ''
    previewImageLoaded.value = false
    previewMaximized.value = false
    previewOpener.value = null
  }

  function captureContext(capture: CaptureItem): PreviewContext {
    const page = pageForCapture(capture, capturePageIndex.value)
    return page ? { pageTitle: page.title || page.url, pageUrl: page.url } : {}
  }

  function captureSiteUrl(capture: CaptureItem): string | null {
    const page = pageForCapture(capture, capturePageIndex.value)
    return page ? externalUrl(page.url) : null
  }

  function isActiveCapture(capture: CaptureItem): boolean {
    const key = captureIdentity(capture)
    return Boolean(key && activeCapture.value && key === captureIdentity(activeCapture.value))
  }

  function activeGalleryElement(): HTMLElement | null {
    return galleryWorkspace.value?.querySelector<HTMLElement>(galleryView.value === 'grid'
      ? '.capture-card.is-active' : '.size-button.is-active') ?? null
  }

  async function revealActiveCapture(): Promise<void> {
    if (galleryView.value === 'grid') {
      const index = visibleCaptures.value.findIndex(isActiveCapture)
      if (index >= 0) capturePage.value = Math.floor(index / CAPTURE_PAGE_SIZE) + 1
    }
    await nextTick()
    activeGalleryElement()?.scrollIntoView?.({ block: 'nearest' })
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
      && selectedCapture.value?.filename === request.filename
      && selectedCapture.value?.sizeId === request.sizeId
      && selectedPageTitle.value === request.pageTitle
      && selectedPageUrl.value === request.pageUrl
  }

  function previewEntryMatches(entry: PreviewEntry, request: PreviewRequest): boolean {
    return entry.capture.path === request.path
      && entry.capture.filename === request.filename
      && (entry.context.sizeId ?? entry.capture.sizeId) === request.sizeId
      && (galleryView.value === 'grid' || (
        (entry.context.pageTitle || '') === request.pageTitle
        && (entry.context.pageUrl || '') === request.pageUrl
      ))
  }

  async function openCapturePreview(capture: CaptureItem, context: PreviewContext = {}, opener?: EventTarget | null): Promise<void> {
    rememberPreviewOpener(opener)
    const request: PreviewRequest = {
      token: ++previewRequestToken,
      path: capture.path,
      filename: capture.filename,
      pageTitle: context.pageTitle || '',
      pageUrl: context.pageUrl || '',
      sizeId: context.sizeId ?? capture.sizeId,
    }
    activePreviewRequest = request
    selectedCapture.value = capture
    activeCapture.value = capture
    activePageUrl.value = request.pageUrl
    activeSizeId.value = request.sizeId
    selectedPageTitle.value = request.pageTitle
    selectedPageUrl.value = request.pageUrl
    selectedImageUrl.value = ''
    previewError.value = context.initialError || ''
    previewPending.value = Boolean(isTauri && capture.path && !context.initialError)
    previewImageLoaded.value = false

    await revealActiveCapture()
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
    return openCapturePreview(capture, captureContext(capture), opener)
  }

  function seoPreviewEntry(page: SeoPageItem, sizeId: string): PreviewEntry {
    const size = gallerySizes.value.find((viewport) => viewport.id === sizeId)
    const label = pageSizeOptions(page, gallerySizes.value).find((option) => option.id === sizeId)?.label || size?.label || sizeId
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
    return {
      capture: capture || fallback,
      context: {
        pageTitle: page.title || page.url,
        pageUrl: page.url,
        sizeId,
        initialError: capture ? undefined : mappedCapture ? 'このページ・サイズの画像パスを取得できませんでした' : 'このページ・サイズのキャプチャは未取得です',
      },
    }
  }

  async function openSeoCapture(page: SeoPageItem, sizeId: string, opener?: EventTarget | null): Promise<void> {
    const entry = seoPreviewEntry(page, sizeId)
    await openCapturePreview(entry.capture, entry.context, opener)
  }

  function navigatePreview(offset: -1 | 1): void {
    const nextEntry = previewEntries.value[selectedPreviewIndex.value + offset]
    if (!nextEntry) return
    void openCapturePreview(nextEntry.capture, nextEntry.context, previewOpener.value)
  }

  function handlePreviewImageError(event: Event): void {
    if (!activePreviewRequest || !isCurrentPreview(activePreviewRequest)) return
    const image = event.currentTarget as HTMLImageElement | null
    if (image && image.src !== selectedImageUrl.value) return
    selectedImageUrl.value = ''
    previewPending.value = false
    previewImageLoaded.value = false
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
    const opener = activeGalleryElement() || previewOpener.value
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
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement) return
      event.preventDefault()
      event.stopPropagation()
      navigatePreview(event.key === 'ArrowLeft' ? -1 : 1)
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

  watch(previewImageMode, (value) => {
    try { localStorage.setItem(PREVIEW_IMAGE_MODE_STORAGE_KEY, value) } catch { /* 表示設定の保存は任意 */ }
  })

  return {
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
  }
}

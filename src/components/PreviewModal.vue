<script setup lang="ts">
import AppIcon from './AppIcon.vue'
import type { CaptureItem } from '../types'
import type { PreviewImageMode } from '../composables/contracts'
import type { usePreview } from '../composables/usePreview'
import type { ComponentPublicInstance } from 'vue'
import { isTauri } from '../bridge'
import { formatBytes } from '../types'
import { externalUrl } from '../urls'

defineProps<{
  openCapture: (capture: CaptureItem) => Promise<void>
  revealCapture: (capture: CaptureItem) => Promise<void>
  openSiteUrl: (url: string) => Promise<void>
  selectedCapture: CaptureItem | null
  selectedPageTitle: string
  displayedPreviewUrl: string
  selectedImageUrl: string
  previewPending: boolean
  previewError: string
  previewImageLoaded: boolean
  previewImage: HTMLImageElement | null
  canShowPreviousPreview: boolean
  canShowNextPreview: boolean
  selectedPreviewIndex: number
  previewEntries: ReturnType<typeof usePreview>['previewEntries']['value']
  closePreview: ReturnType<typeof usePreview>['closePreview']
  onModalKeydown: ReturnType<typeof usePreview>['onModalKeydown']
  handlePreviewImageLoad: ReturnType<typeof usePreview>['handlePreviewImageLoad']
  handlePreviewImageError: ReturnType<typeof usePreview>['handlePreviewImageError']
  retryPreview: ReturnType<typeof usePreview>['retryPreview']
  navigatePreview: ReturnType<typeof usePreview>['navigatePreview']
  setPreviewModal: (element: Element | ComponentPublicInstance | null) => void
  setPreviewImage: (element: Element | ComponentPublicInstance | null) => void
}>()

const previewMaximized = defineModel<boolean>('previewMaximized', { required: true })

const previewImageMode = defineModel<PreviewImageMode>('previewImageMode', { required: true })
</script>

<template>
  <div
    v-if="selectedCapture"
    class="preview-backdrop"
    :class="{ 'is-maximized': previewMaximized }"
    role="presentation"
    @click.self="closePreview"
  >
    <section
      :ref="setPreviewModal"
      class="preview-modal"
      :class="{ 'is-maximized': previewMaximized }"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      :aria-label="`${selectedPageTitle || selectedCapture.filename}のプレビュー`"
      @keydown="onModalKeydown"
    >
      <header>
        <div>
          <h2>{{ selectedPageTitle || selectedCapture.filename }}</h2>
          <a
            v-if="externalUrl(displayedPreviewUrl)"
            class="preview-url"
            :href="externalUrl(displayedPreviewUrl)!"
            :title="displayedPreviewUrl"
            @click.prevent="openSiteUrl(displayedPreviewUrl)"
          >
            <span>{{ displayedPreviewUrl }}</span>
            <AppIcon name="external" />
          </a>
          <p v-else-if="displayedPreviewUrl" class="preview-url">{{ displayedPreviewUrl }}</p>
          <p class="preview-meta">
            <span class="preview-filename" :title="selectedCapture.filename">{{ selectedCapture.filename }}</span>
            <span class="preview-size"> · {{ selectedCapture.sizeLabel || 'サイズ未特定' }} · {{ selectedCapture.width || '—' }} × {{ selectedCapture.height || '—' }}</span>
            <span class="preview-bytes"> · {{ formatBytes(selectedCapture.bytes) }}</span>
          </p>
        </div>
        <button
          type="button"
          :aria-label="previewMaximized ? '元のサイズに戻す' : 'ウィンドウいっぱいに表示'"
          :title="previewMaximized ? '元のサイズに戻す' : 'ウィンドウいっぱいに表示'"
          :aria-pressed="previewMaximized"
          @click="previewMaximized = !previewMaximized"
        >
          <AppIcon :name="previewMaximized ? 'restore' : 'maximize'" />
        </button>
        <button type="button" aria-label="プレビューを閉じる" @click="closePreview">
          <AppIcon name="close" />
        </button>
      </header>
      <div class="preview-toolbar">
        <div
          v-if="selectedImageUrl || previewPending || previewError"
          class="preview-controls"
          aria-label="画像表示"
        >
          <span>表示:</span>
          <button
            type="button"
            :disabled="!previewImageLoaded || !!previewError"
            :class="{ active: previewImageMode === 'fit' }"
            :aria-pressed="previewImageMode === 'fit'"
            @click="previewImageMode = 'fit'"
          >全体</button>
          <button
            type="button"
            :disabled="!previewImageLoaded || !!previewError"
            :class="{ active: previewImageMode === 'actual' }"
            :aria-pressed="previewImageMode === 'actual'"
            @click="previewImageMode = 'actual'"
          >100%</button>
          <button
            type="button"
            :disabled="!previewImageLoaded || !!previewError"
            :class="{ active: previewImageMode === 'width' }"
            :aria-pressed="previewImageMode === 'width'"
            @click="previewImageMode = 'width'"
          >幅100%</button>
          <span v-if="previewImageLoaded && previewImage" class="preview-natural-size">{{ previewImage.naturalWidth }} × {{ previewImage.naturalHeight }}px</span>
        </div>
        <div class="preview-actions">
          <button
            class="primary"
            type="button"
            :disabled="!isTauri || !selectedCapture.path"
            @click="openCapture(selectedCapture)"
          >元画像を開く</button>
          <button
            type="button"
            :disabled="!isTauri || !selectedCapture.path"
            @click="revealCapture(selectedCapture)"
          >Finderで表示</button>
        </div>
      </div>
      <div class="preview-image-area">
        <div
          class="preview-image-wrap"
          :class="{ 'is-actual': previewImageMode === 'actual', 'is-width': previewImageMode === 'width' }"
        >
          <div
            v-if="previewPending"
            class="preview-loading"
            role="status"
            aria-live="polite"
          >読み込み中…</div>
          <img
            v-else-if="selectedImageUrl"
            :ref="setPreviewImage"
            :style="previewImageMode === 'width' && previewImageLoaded && previewImage ? { width: `min(100%, ${previewImage.naturalWidth}px)` } : undefined"
            :src="selectedImageUrl"
            :alt="`${selectedCapture.filename}のプレビュー`"
            @load="handlePreviewImageLoad"
            @error="handlePreviewImageError"
          />
          <div v-else-if="previewError" class="preview-error">
            <strong>プレビューを読み込めませんでした</strong>
            <span>{{ previewError }}</span>
            <small>元画像を開くか、Finderで表示してください。</small>
            <button v-if="selectedCapture.path" type="button" @click="retryPreview">再試行</button>
          </div>
          <div v-else-if="!isTauri" class="mock-page large">
            <span></span>
            <i></i>
            <b></b>
            <em></em>
          </div>
          <div v-else class="preview-error">
            <strong>プレビュー画像がありません</strong>
            <span>このキャプチャには表示可能な画像がありません。</span>
          </div>
        </div>
        <button
          class="preview-arrow is-previous"
          type="button"
          :disabled="!canShowPreviousPreview"
          aria-label="前の画像"
          title="前の画像（←）"
          @click="navigatePreview(-1)"
        >
          <AppIcon name="previous" />
        </button>
        <button
          class="preview-arrow is-next"
          type="button"
          :disabled="!canShowNextPreview"
          aria-label="次の画像"
          title="次の画像（→）"
          @click="navigatePreview(1)"
        >
          <AppIcon name="next" />
        </button>
      </div>
      <footer>
        <nav class="preview-navigation" aria-label="画像の移動">
          <span v-if="selectedPreviewIndex >= 0" aria-live="polite">{{ selectedPreviewIndex + 1 }} / {{ previewEntries.length }}</span>
        </nav>
      </footer>
    </section>
  </div>
</template>

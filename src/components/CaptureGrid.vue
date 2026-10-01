<script setup lang="ts">
import AppIcon from './AppIcon.vue'
import type { CaptureItem } from '../types'
import type { usePreview } from '../composables/usePreview'
import { isTauri } from '../bridge'
import { formatBytes } from '../types'

defineProps<{
  pagedVisibleCaptures: CaptureItem[]
  isActiveCapture: ReturnType<typeof usePreview>['isActiveCapture']
  openPreview: ReturnType<typeof usePreview>['openPreview']
  captureSiteUrl: ReturnType<typeof usePreview>['captureSiteUrl']
  capturePreview: (capture: CaptureItem) => string
  captureImage: (capture: CaptureItem) => string
  captureThumbnailLoading: (capture: CaptureItem) => boolean
  openCapture: (capture: CaptureItem) => Promise<void>
  revealCapture: (capture: CaptureItem) => Promise<void>
  openSiteUrl: (url: string) => Promise<void>
}>()
</script>

<template>
  <div class="capture-grid">
    <article
      v-for="capture in pagedVisibleCaptures"
      :key="capture.path || `${capture.filename}-${capture.sizeId || 'size'}`"
      class="capture-card"
      :class="{ 'is-active': isActiveCapture(capture) }"
      :aria-current="isActiveCapture(capture) ? 'true' : undefined"
      tabindex="0"
      @click="openPreview(capture, $event.currentTarget)"
      @dblclick="openCapture(capture)"
      @keydown.self.enter.prevent="openPreview(capture, $event.currentTarget)"
      @keydown.self.space.prevent="openPreview(capture, $event.currentTarget)"
    >
      <div class="capture-thumb" :style="{ background: capturePreview(capture) }">
        <img
          v-if="captureImage(capture)"
          :src="captureImage(capture)"
          :alt="`${capture.filename}のプレビュー`"
        />
        <div v-else-if="!isTauri" class="mock-page">
          <span></span>
          <i></i>
          <b></b>
          <em></em>
        </div>
        <div v-else-if="captureThumbnailLoading(capture)" class="capture-thumb-state" role="status">画像を読み込み中…</div>
        <div v-else class="capture-thumb-state">画像未取得</div>
      </div>
      <div class="capture-meta">
        <div class="capture-file">
          <AppIcon
            class="device-icon"
            :name="capture.width && capture.width > 1000 ? 'desktop' : 'mobile'"
          />
          <span class="truncate">{{ capture.filename }}</span>
        </div>
        <div class="capture-tags">
          <span class="size-tag" :class="capture.sizeId">{{ capture.sizeLabel || 'サイズ' }}</span>
          <span>{{ capture.width }} × {{ capture.height }}</span>
        </div>
        <div class="capture-bottom">
          <span>{{ formatBytes(capture.bytes) }}</span>
          <span class="capture-actions">
            <button
              type="button"
              @click.stop="openSiteUrl(captureSiteUrl(capture)!)"
              @dblclick.stop
              :disabled="!captureSiteUrl(capture)"
              aria-label="サイトを開く"
              title="サイトを開く"
            >
              <AppIcon name="globe" />
            </button>
            <button
              type="button"
              @click.stop="openCapture(capture)"
              :disabled="!isTauri"
              aria-label="元画像を開く"
            >
              <AppIcon name="external" />
            </button>
            <button
              type="button"
              @click.stop="revealCapture(capture)"
              :disabled="!isTauri"
              aria-label="フォルダで表示"
            >
              <AppIcon name="folder" />
            </button>
          </span>
        </div>
      </div>
    </article>
  </div>
</template>

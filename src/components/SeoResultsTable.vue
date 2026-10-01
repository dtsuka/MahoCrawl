<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { externalUrl } from '../urls'
import type { CaptureViewport, SeoPageItem } from '../types'

const props = defineProps<{
  pages: SeoPageItem[]
  sizes: CaptureViewport[]
  activePageUrl?: string
  activeSizeId?: string | null
}>()

const emit = defineEmits<{
  (event: 'open-url', url: string): void
  (event: 'open-capture', page: SeoPageItem, sizeId: string, opener: EventTarget | null): void
}>()

const expandedUrls = ref<Set<string>>(new Set())

const columns = [
  { id: 'url', label: 'URL / ページ', width: 270, compactWidth: 230, minWidth: 160 },
  { id: 'captures', label: 'キャプチャサイズ / 状態', width: 190, compactWidth: 175, minWidth: 140 },
  { id: 'title', label: 'Title', width: 210, compactWidth: 190, minWidth: 100 },
  { id: 'description', label: 'Description', width: 270, compactWidth: 240, minWidth: 120 },
  { id: 'canonical', label: 'Canonical', width: 230, compactWidth: 205, minWidth: 120 },
  { id: 'metadata', label: 'OGP / Twitter', width: 320, compactWidth: 290, minWidth: 140 },
  { id: 'headings', label: 'H1 / H2', width: 200, compactWidth: 180, minWidth: 100 },
] as const

type Column = typeof columns[number]
type ColumnWidths = Partial<Record<Column['id'], number>>
const COLUMN_WIDTHS_STORAGE_KEY = 'maho-crawl-seo-column-widths-v1'
const MAX_COLUMN_WIDTH = 2000
const compactMedia = window.matchMedia?.('(max-width: 760px)')
const compact = ref(compactMedia?.matches ?? false)
const columnWidths = ref<ColumnWidths>(readColumnWidths())
const tableWidth = computed(() => columns.reduce((total, column) => total + columnWidth(column), 0))
const resizing = ref<{
  column: Column
  pointerId: number
  startX: number
  startWidth: number
  handle: HTMLElement
} | null>(null)

function clampWidth(column: Column, width: number): number {
  return Math.round(Math.max(column.minWidth, Math.min(MAX_COLUMN_WIDTH, width)))
}

function readColumnWidths(): ColumnWidths {
  try {
    const saved = JSON.parse(localStorage.getItem(COLUMN_WIDTHS_STORAGE_KEY) || '{}')
    const widths: ColumnWidths = {}
    for (const column of columns) {
      const width = saved?.[column.id]
      if (typeof width === 'number' && Number.isFinite(width)) widths[column.id] = clampWidth(column, width)
    }
    return widths
  } catch {
    return {}
  }
}

function saveColumnWidths(): void {
  try { localStorage.setItem(COLUMN_WIDTHS_STORAGE_KEY, JSON.stringify(columnWidths.value)) } catch { /* preference is optional */ }
}

function columnWidth(column: Column): number {
  return columnWidths.value[column.id] ?? (compact.value ? column.compactWidth : column.width)
}

function startResize(column: Column, event: PointerEvent): void {
  if (event.button !== 0 || resizing.value) return
  event.preventDefault()
  const handle = event.currentTarget as HTMLElement
  handle.focus({ preventScroll: true })
  handle.setPointerCapture(event.pointerId)
  resizing.value = { column, pointerId: event.pointerId, startX: event.clientX, startWidth: columnWidth(column), handle }
}

function resizeColumn(event: PointerEvent): void {
  const drag = resizing.value
  if (!drag || event.pointerId !== drag.pointerId) return
  columnWidths.value[drag.column.id] = clampWidth(drag.column, drag.startWidth + event.clientX - drag.startX)
}

function finishResize(): void {
  const drag = resizing.value
  if (!drag) return
  resizing.value = null
  if (drag.handle.hasPointerCapture(drag.pointerId)) drag.handle.releasePointerCapture(drag.pointerId)
  saveColumnWidths()
}

function endResize(event: PointerEvent): void {
  if (event.pointerId === resizing.value?.pointerId) finishResize()
}

function resetColumnWidth(column: Column): void {
  delete columnWidths.value[column.id]
  saveColumnWidths()
}

function resizeWithKeyboard(column: Column, event: KeyboardEvent): void {
  if (event.key === 'Enter') {
    event.preventDefault()
    resetColumnWidth(column)
  } else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    event.preventDefault()
    const step = event.shiftKey ? 50 : 10
    columnWidths.value[column.id] = clampWidth(column, columnWidth(column) + (event.key === 'ArrowRight' ? step : -step))
    saveColumnWidths()
  }
}

function updateCompact(event: MediaQueryListEvent): void {
  compact.value = event.matches
}

compactMedia?.addEventListener('change', updateCompact)
window.addEventListener('blur', finishResize)
onBeforeUnmount(() => {
  finishResize()
  compactMedia?.removeEventListener('change', updateCompact)
  window.removeEventListener('blur', finishResize)
})

interface PageSizeOption {
  id: string
  label: string
}

function valueOrMissing(value: string | null | undefined): string {
  return value?.trim() || '未取得'
}

function hasValue(value: string | null | undefined): boolean {
  return Boolean(value?.trim())
}

function pageSizeOptions(page: SeoPageItem): PageSizeOption[] {
  return page.sizeIds.map((id, index) => ({
    id,
    label: page.captureBySize?.[id]?.sizeLabel?.trim()
      || props.sizes.find((size) => size.id === id)?.label?.trim()
      || (page.sizeIds.length === page.sizeLabels.length ? page.sizeLabels[index]?.trim() : '')
      || id,
  }))
}

function isExpanded(url: string): boolean {
  return expandedUrls.value.has(url)
}

function detailId(index: number): string {
  return `seo-detail-${index}`
}

function toggleDetails(url: string): void {
  const next = new Set(expandedUrls.value)
  if (next.has(url)) next.delete(url)
  else next.add(url)
  expandedUrls.value = next
}

function openCapture(page: SeoPageItem, sizeId: string, event: MouseEvent): void {
  emit('open-capture', page, sizeId, event.currentTarget)
}

function metadataValues(page: SeoPageItem): Array<{ label: string; value: string | null }> {
  return [
    { label: 'og:title', value: page.ogTitle },
    { label: 'og:description', value: page.ogDescription },
    { label: 'og:image', value: page.ogImage },
    { label: 'twitter:title', value: page.twitterTitle },
    { label: 'twitter:description', value: page.twitterDescription },
    { label: 'twitter:image', value: page.twitterImage },
  ]
}

function headingValues(page: SeoPageItem): Array<{ label: string; value: string | null }> {
  return [
    { label: 'H1', value: page.h1 },
    { label: 'H2', value: page.h2 },
  ]
}
</script>

<template>
  <div class="seo-results-table" :class="{ 'is-resizing': resizing }">
    <div class="table-scroll" tabindex="0" aria-label="SEO結果テーブル。横方向にスクロールできます">
      <table class="results-table" :style="{ width: `${tableWidth}px`, '--url-column-width': `${columnWidth(columns[0])}px` }">
        <caption class="sr-only">クロールしたページのSEO概要</caption>
        <colgroup>
          <col v-for="column in columns" :key="column.id" :class="`column-${column.id}`" :style="{ width: `${columnWidth(column)}px` }" />
        </colgroup>
        <thead>
          <tr>
            <th v-for="column in columns" :key="column.id" scope="col">
              <span class="column-label" :title="column.label">{{ column.label }}</span>
              <span
                class="column-resizer"
                :class="{ active: resizing?.column.id === column.id }"
                role="separator"
                tabindex="0"
                aria-orientation="vertical"
                :aria-label="`${column.label}列の幅を調整`"
                :aria-valuemin="column.minWidth"
                :aria-valuemax="MAX_COLUMN_WIDTH"
                :aria-valuenow="columnWidth(column)"
                :aria-valuetext="`${columnWidth(column)}ピクセル`"
                aria-description="左右の矢印キーで幅を調整、Enterキーで元の幅に戻します"
                title="ドラッグで列幅を調整・ダブルクリックで元の幅に戻す"
                @pointerdown="startResize(column, $event)"
                @pointermove="resizeColumn"
                @pointerup="endResize"
                @pointercancel="endResize"
                @lostpointercapture="endResize"
                @dblclick.prevent="resetColumnWidth(column)"
                @keydown="resizeWithKeyboard(column, $event)"
              ></span>
            </th>
          </tr>
        </thead>
        <tbody>
          <template v-for="(page, index) in props.pages" :key="page.url">
            <tr class="result-row" :class="{ 'is-active': page.url === activePageUrl }">
              <th scope="row" class="url-cell">
                <a v-if="externalUrl(page.url)" class="cell-text url-text url-link" :href="externalUrl(page.url)!" :title="page.url" @click.prevent="emit('open-url', externalUrl(page.url)!)">{{ page.url.trim() }}</a>
                <span v-else class="cell-text url-text" :title="valueOrMissing(page.url)">{{ valueOrMissing(page.url) }}</span>
                <button
                  class="detail-toggle"
                  type="button"
                  :aria-expanded="isExpanded(page.url)"
                  :aria-controls="detailId(index)"
                  @click="toggleDetails(page.url)"
                >
                  <span>{{ isExpanded(page.url) ? '詳細を閉じる' : '詳細' }}</span>
                  <svg class="detail-chevron" viewBox="0 0 12 12" aria-hidden="true">
                    <path d="M3 4.5 6 7.5 9 4.5" />
                  </svg>
                </button>
              </th>
              <td>
                <div v-if="pageSizeOptions(page).length" class="size-list">
                  <button
                    v-for="size in pageSizeOptions(page)"
                    :key="size.id"
                    class="size-button"
                    :class="{ 'is-active': page.url === activePageUrl && size.id === activeSizeId }"
                    type="button"
                    :aria-label="`${valueOrMissing(page.url)}の${size.label}キャプチャを表示`"
                    @click.stop="openCapture(page, size.id, $event)"
                  >
                    <svg class="image-icon" viewBox="0 0 16 16" aria-hidden="true">
                      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
                      <circle cx="5.5" cy="6" r="1.1" />
                      <path d="m3.5 11 2.6-2.6 2.1 2 1.5-1.5 2.8 2.8" />
                    </svg>
                    <span>{{ size.label }}</span>
                  </button>
                </div>
                <span v-else class="missing">{{ props.sizes.length ? '未取得' : 'キャプチャなし' }}</span>
                <span v-if="hasValue(page.status)" class="status-text">{{ page.status }}</span>
                <span v-else class="missing status-text">状態未取得</span>
              </td>
              <td>
                <span class="cell-text" :class="{ missing: !hasValue(page.title) }" :title="valueOrMissing(page.title)">{{ valueOrMissing(page.title) }}</span>
              </td>
              <td>
                <span class="cell-text" :class="{ missing: !hasValue(page.description) }" :title="valueOrMissing(page.description)">{{ valueOrMissing(page.description) }}</span>
              </td>
              <td>
                <span class="cell-text" :class="{ missing: !hasValue(page.canonical) }" :title="valueOrMissing(page.canonical)">{{ valueOrMissing(page.canonical) }}</span>
              </td>
              <td>
                <div v-if="metadataValues(page).some((entry) => hasValue(entry.value))" class="metadata-list">
                  <span v-for="entry in metadataValues(page)" v-show="hasValue(entry.value)" :key="entry.label" class="metadata-line" :title="`${entry.label}: ${valueOrMissing(entry.value)}`">
                    <span class="metadata-label">{{ entry.label }}:</span> {{ valueOrMissing(entry.value) }}
                  </span>
                </div>
                <span v-else class="missing">未取得</span>
              </td>
              <td>
                <div v-if="headingValues(page).some((entry) => hasValue(entry.value))" class="metadata-list">
                  <span v-for="entry in headingValues(page)" v-show="hasValue(entry.value)" :key="entry.label" class="metadata-line" :title="`${entry.label}: ${valueOrMissing(entry.value)}`">
                    <span class="metadata-label">{{ entry.label }}:</span> {{ valueOrMissing(entry.value) }}
                  </span>
                </div>
                <span v-else class="missing">未取得</span>
              </td>
            </tr>
            <tr v-if="isExpanded(page.url)" :id="detailId(index)" class="detail-row">
              <td colspan="7">
                <section class="detail-panel" :aria-labelledby="`${detailId(index)}-heading`">
                  <h3 :id="`${detailId(index)}-heading`">ページ詳細</h3>
                  <dl class="detail-grid">
                    <div class="detail-entry detail-entry-wide">
                      <dt>URL</dt>
                      <dd :class="{ missing: !hasValue(page.url) }">{{ valueOrMissing(page.url) }}</dd>
                    </div>
                    <div class="detail-entry">
                      <dt>Title</dt>
                      <dd :class="{ missing: !hasValue(page.title) }">{{ valueOrMissing(page.title) }}</dd>
                    </div>
                    <div class="detail-entry detail-entry-wide">
                      <dt>Description</dt>
                      <dd :class="{ missing: !hasValue(page.description) }">{{ valueOrMissing(page.description) }}</dd>
                    </div>
                    <div class="detail-entry detail-entry-wide">
                      <dt>Canonical</dt>
                      <dd :class="{ missing: !hasValue(page.canonical) }">{{ valueOrMissing(page.canonical) }}</dd>
                    </div>
                    <div v-for="entry in metadataValues(page)" :key="entry.label" class="detail-entry">
                      <dt>{{ entry.label }}</dt>
                      <dd :class="{ missing: !hasValue(entry.value) }">{{ valueOrMissing(entry.value) }}</dd>
                    </div>
                    <div v-for="entry in headingValues(page)" :key="entry.label" class="detail-entry">
                      <dt>{{ entry.label }}</dt>
                      <dd :class="{ missing: !hasValue(entry.value) }">{{ valueOrMissing(entry.value) }}</dd>
                    </div>
                  </dl>
                </section>
              </td>
            </tr>
          </template>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.seo-results-table {
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1 1 auto;
  color: #465364;
  font-size: 13px;
  line-height: 1.45;
}

.table-scroll {
  min-width: 0;
  max-width: 100%;
  flex: 1 1 auto;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  container: seo-table / inline-size;
}

.results-table {
  border-spacing: 0;
  border-collapse: separate;
  table-layout: fixed;
  background: #fff;
}

.column-label {
  display: block;
  overflow: hidden;
  padding-right: 5px;
  text-overflow: ellipsis;
}

.column-resizer {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 10px;
  cursor: col-resize;
  touch-action: none;
  user-select: none;
}

.column-resizer::after {
  position: absolute;
  top: 9px;
  right: 0;
  bottom: 9px;
  width: 2px;
  background: #d3dce7;
  content: '';
}

.column-resizer:hover::after,
.column-resizer:focus-visible::after,
.column-resizer.active::after {
  background: #2878ed;
}

.column-resizer:focus-visible {
  outline: 2px solid #2878ed;
  outline-offset: -2px;
}

.is-resizing,
.is-resizing .detail-entry dd {
  cursor: col-resize;
  user-select: none;
}

.results-table th,
.results-table td {
  min-width: 0;
  padding: 12px 13px;
  border-bottom: 1px solid #e7ebf0;
  vertical-align: top;
  text-align: left;
}

.results-table thead th {
  position: sticky;
  z-index: 3;
  top: 0;
  background: #f8fafc;
  color: #607083;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: .01em;
  white-space: nowrap;
}

.results-table thead th:first-child {
  left: 0;
  left: min(0px, calc(60cqi - var(--url-column-width)));
  z-index: 5;
}

.results-table tbody th {
  color: #29384a;
  font-weight: 600;
}

.results-table tbody tr.result-row:hover > th,
.results-table tbody tr.result-row:hover > td {
  background: #fbfdff;
}

.url-cell {
  position: sticky;
  z-index: 2;
  left: 0;
  /* Let a wide URL column scroll until its resize handle and other columns are reachable. */
  left: min(0px, calc(60cqi - var(--url-column-width)));
  background: #fff;
  box-shadow: 7px 0 10px -12px rgba(30, 56, 88, .55);
}

.cell-text {
  display: block;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.url-text {
  color: #244f88;
  overflow-wrap: anywhere;
}

.missing {
  color: #5f6f81;
  font-size: 12px;
  font-weight: 500;
}

.size-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.size-button {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 28px;
  max-width: 100%;
  padding: 4px 8px;
  border: 1px solid #c4d8f5;
  border-radius: 5px;
  background: #eef5ff;
  color: #1e5dad;
  font: inherit;
  font-size: 12px;
  font-weight: 650;
  line-height: 1.2;
  cursor: pointer;
  text-align: left;
  transition: border-color .15s ease, background-color .15s ease, color .15s ease, box-shadow .15s ease;
}

.results-table tbody tr.result-row.is-active > th,
.results-table tbody tr.result-row.is-active > td { background: #eef5ff; }

.size-button.is-active { border-color: #2878ed; background: #d7e7ff; box-shadow: 0 0 0 1px #2878ed; }
.url-link { color: #2367bd; text-decoration: none; }
.url-link:hover { text-decoration: underline; text-underline-offset: 3px; }
.url-link:focus-visible { outline: 2px solid #2878ed; outline-offset: 2px; }

.size-button:hover {
  border-color: #8fb7ec;
  background: #e1edff;
  color: #124e9c;
}

.size-button:focus-visible,
.detail-toggle:focus-visible,
.table-scroll:focus-visible {
  outline: 2px solid #2878ed;
  outline-offset: 2px;
}

.size-button:active {
  background: #d7e7ff;
}

.image-icon {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.35;
}

.status-text {
  display: block;
  margin-top: 8px;
  color: #6f7c8b;
  font-size: 12px;
}

.metadata-list {
  display: grid;
  gap: 5px;
  min-width: 0;
}

.metadata-line {
  display: block;
  max-width: 100%;
  overflow: hidden;
  color: #526273;
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.metadata-label {
  color: #526273;
  font-weight: 650;
}

.detail-toggle {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 8px;
  padding: 0;
  border: 0;
  background: transparent;
  color: #2367bd;
  font-size: 12px;
  font-weight: 650;
  cursor: pointer;
}

.detail-toggle:hover {
  color: #124e9c;
  text-decoration: underline;
  text-underline-offset: 3px;
}

.detail-chevron {
  width: 12px;
  height: 12px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.5;
  transition: transform .15s ease;
}

.detail-toggle[aria-expanded="true"] .detail-chevron {
  transform: rotate(180deg);
}

.detail-row > td {
  position: relative;
  padding: 0;
  background: #f8fafc;
}

.detail-panel {
  position: sticky;
  z-index: 4;
  left: 0;
  box-sizing: border-box;
  width: 100cqi;
  max-width: 100cqi;
  min-width: 0;
  padding: 17px 18px 19px;
  border-bottom: 1px solid #dfe6ee;
  background: #f8fafc;
}

@supports not (width: 100cqi) {
  .detail-panel {
    width: 100%;
    max-width: 100%;
  }
}

.detail-panel h3 {
  margin: 0 0 13px;
  color: #33455b;
  font-size: 13px;
  font-weight: 700;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px 18px;
  margin: 0;
}

.detail-entry {
  min-width: 0;
}

.detail-entry-wide {
  grid-column: span 2;
}

.detail-entry dt {
  margin-bottom: 4px;
  color: #526273;
  font-size: 12px;
  font-weight: 700;
}

.detail-entry dd {
  min-width: 0;
  margin: 0;
  color: #36495e;
  font-size: 13px;
  line-height: 1.5;
  overflow-wrap: anywhere;
  user-select: text;
  white-space: pre-wrap;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

@media (max-width: 760px) {
  .results-table th,
  .results-table td { padding: 11px 12px; }
}

@container seo-table (max-width: 620px) {
  .detail-grid { display: block; }
  .detail-panel { padding: 15px 14px 17px; }
  .detail-entry { margin-top: 13px; }
  .detail-entry:first-child { margin-top: 0; }
}

@media (prefers-reduced-motion: reduce) {
  .size-button,
  .detail-chevron { transition: none; }
}
</style>

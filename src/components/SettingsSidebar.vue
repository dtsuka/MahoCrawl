<script setup lang="ts">
import { reactive } from 'vue'
import { DEFAULT_CONFIGURATION } from '../types'
import { selectOutputFolder } from '../bridge'
import AppIcon from './AppIcon.vue'
import type { CrawlConfiguration, CaptureViewport } from '../types'
import type { SettingsSection } from '../composables/contracts'
import type { useCrawlRun } from '../composables/useCrawlRun'
import type { ComponentPublicInstance } from 'vue'
import { isTauri } from '../bridge'

const { configuration, sectionOpen } = defineProps<{
  sidebarOpen: boolean
  configuration: CrawlConfiguration
  controlsDisabled: boolean
  sectionOpen: Record<SettingsSection, boolean>
  metadataOnly: boolean
  isBusy: boolean
  canStart: boolean
  starting: boolean
  retryableRun: boolean
  savingConfiguration: boolean
  storageError: string
  saveError: string
  engineVersion: string
  beginCrawl: ReturnType<typeof useCrawlRun>['beginCrawl']
  cancelCrawl: ReturnType<typeof useCrawlRun>['cancelCrawl']
  setSidebarElement: (element: Element | ComponentPublicInstance | null) => void
  setTargetUrlInput: (element: Element | ComponentPublicInstance | null) => void
}>()

const emit = defineEmits<{
  error: [message: string]
  'size-removed': [id: string]
}>()

const newSize = reactive({ label: '', width: 1280, height: 800 })

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
    emit('error', 'サイズ名・幅・高さを入力してください。')
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
  emit('error', '')
}

function removeSize(id: string): void {
  configuration.captures = configuration.captures.filter((capture) => capture.id !== id)
  emit('size-removed', id)
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
    emit('error', String(error))
  }
}
</script>

<template>
  <aside
    :ref="setSidebarElement"
    class="settings-sidebar"
    :class="{ 'sidebar-open': sidebarOpen }"
    aria-label="クロール設定"
  >
    <div class="settings-scroll">
      <section class="setting-section">
        <h2 class="section-heading" id="target-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.target"
            aria-controls="target-settings"
            @click="toggleSection('target')"
          >
            <span class="section-title">クロール対象</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="target-settings" v-show="sectionOpen.target" class="section-content">
          <label class="field-label" for="target-url">URL</label>
          <input
            id="target-url"
            :ref="setTargetUrlInput"
            v-model="configuration.targetUrl"
            class="text-input"
            type="url"
            placeholder="https://example.com"
            :disabled="controlsDisabled"
          />
          <label class="switch-row">
            <input
              v-model="configuration.singlePage"
              type="checkbox"
              :disabled="controlsDisabled"
            />
            <span class="switch" aria-hidden="true"></span>
            <span>このページのみ</span>
          </label>
          <label class="field-label" for="user-agent">ユーザーエージェント（カスタム）</label>
          <input
            id="user-agent"
            v-model="configuration.userAgent"
            class="text-input"
            type="text"
            placeholder="空欄ならDesktop"
            :disabled="controlsDisabled"
          />
          <div class="field-grid auth-fields">
            <label class="field-label" for="http-auth-user">Basic認証（ユーザー名）
                <input
                id="http-auth-user"
                v-model="configuration.httpAuthUser"
                class="text-input"
                type="text"
                autocomplete="off"
                spellcheck="false"
                placeholder="任意"
                aria-describedby="http-auth-note"
                :disabled="controlsDisabled"
              /></label>
            <label class="field-label" for="http-auth-password">パスワード
                <input
                id="http-auth-password"
                v-model="configuration.httpAuthPassword"
                class="text-input"
                type="password"
                autocomplete="off"
                placeholder="任意"
                aria-describedby="http-auth-note"
                :disabled="controlsDisabled"
              /></label>
          </div>
          <p id="http-auth-note" class="path-note">空欄なら認証なし。パスワードは保存せず、実行時だけ対象URLのoriginへ渡します。</p>
        </div>
      </section>
      <section class="setting-section sizes-section">
        <h2 class="section-heading" id="capture-size-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.sizes"
            aria-controls="capture-size-settings"
            @click="toggleSection('sizes')"
          >
            <span class="section-title">キャプチャサイズ</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="capture-size-settings" v-show="sectionOpen.sizes" class="section-content">
          <p class="section-help">有効なサイズを順番に撮影します。すべてオフ、または削除するとメタ情報のみ取得します。</p>
          <p v-if="metadataOnly" class="field-note" role="status">キャプチャなしでHTML内のメタ情報を取得します。JavaScriptは実行しません。</p>
          <div v-for="viewport in configuration.captures" :key="viewport.id" class="size-row">
            <label class="checkbox-label" :title="`${viewport.label}を有効化`">
              <input
                v-model="viewport.enabled"
                type="checkbox"
                :disabled="controlsDisabled"
                :aria-label="`${displayedLabel(viewport)}を有効化`"
              />
              <span class="checkmark" aria-hidden="true">
                <AppIcon name="check" />
              </span>
            </label>
            <div class="size-fields">
              <input
                :value="viewport.label"
                @input="updateLabel(viewport, $event)"
                class="size-label-input"
                :disabled="controlsDisabled"
                :aria-label="`${displayedLabel(viewport)}の名前`"
              />
              <div class="dimension-fields">
                <input
                  :value="displayedDimension(viewport, 'width')"
                  @input="updateDimension(viewport, 'width', $event)"
                  class="dimension-input"
                  type="number"
                  min="320"
                  max="8192"
                  :disabled="controlsDisabled"
                  :aria-label="`${displayedLabel(viewport)}の幅`"
                />
                <span>×</span>
                <input
                  :value="displayedDimension(viewport, 'height')"
                  @input="updateDimension(viewport, 'height', $event)"
                  class="dimension-input"
                  type="number"
                  min="320"
                  max="8192"
                  :disabled="controlsDisabled"
                  :aria-label="`${displayedLabel(viewport)}の高さ`"
                />
              </div>
            </div>
            <button
              class="icon-button delete"
              type="button"
              :disabled="controlsDisabled"
              :aria-label="`${displayedLabel(viewport)}を削除`"
              @click="removeSize(viewport.id)"
            >
              <AppIcon name="close" />
            </button>
          </div>
          <div class="add-size-row">
            <input
              v-model="newSize.label"
              class="size-label-input"
              type="text"
              placeholder="新しいサイズ"
              aria-label="新しいサイズ名"
              :disabled="controlsDisabled"
              @keydown.enter="addSize"
            />
            <input
              v-model.number="newSize.width"
              class="dimension-input"
              type="number"
              min="320"
              max="8192"
              aria-label="新しいサイズの幅"
              :disabled="controlsDisabled"
            />
            <span>×</span>
            <input
              v-model.number="newSize.height"
              class="dimension-input"
              type="number"
              min="320"
              max="8192"
              aria-label="新しいサイズの高さ"
              :disabled="controlsDisabled"
            />
            <button
              class="add-button"
              type="button"
              :disabled="controlsDisabled"
              @click="addSize"
            ><AppIcon name="plus" />追加</button>
          </div>
        </div>
      </section>
      <section class="setting-section">
        <h2 class="section-heading" id="capture-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.capture"
            aria-controls="capture-settings"
            @click="toggleSection('capture')"
          >
            <span class="section-title">撮影設定</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="capture-settings" v-show="sectionOpen.capture" class="section-content">
          <div class="field-grid">
            <label class="field-label">撮影モード
                <select
                v-model="configuration.screenshotMode"
                class="select-input"
                :disabled="controlsDisabled || metadataOnly"
              >
                <option value="viewport">表示領域</option>
                <option value="full-page">ページ全体</option>
              </select></label>
            <label class="field-label">出力フォーマット
                <select
                v-model="configuration.screenshotFormat"
                class="select-input"
                :disabled="controlsDisabled || metadataOnly"
              >
                <option value="png">PNG</option>
                <option value="jpg">JPG</option>
                <option value="webp">WebP</option>
              </select></label>
          </div>
          <label class="switch-row">
            <input
              v-model="configuration.hideCookieBanner"
              type="checkbox"
              disabled
              aria-describedby="cookie-banner-note"
            />
            <span class="switch" aria-hidden="true"></span>
            <span>Cookieバナー自動非表示（現在未対応）</span>
          </label>
          <p id="cookie-banner-note" class="field-note">設定値は保存されますが、現在のCLIでは自動非表示を実行しません。</p>
        </div>
      </section>
      <section class="setting-section">
        <h2 class="section-heading" id="crawl-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.crawl"
            aria-controls="crawl-settings"
            @click="toggleSection('crawl')"
          >
            <span class="section-title">クロール設定</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="crawl-settings" v-show="sectionOpen.crawl" class="section-content advanced-fields">
          <label class="field-label">最大深度<input
              v-model.number="configuration.maxDepth"
              class="number-input"
              type="number"
              min="0"
              max="20"
              :disabled="controlsDisabled || configuration.singlePage"
            /></label>
          <label class="field-label">同時ワーカー数<input
              v-model.number="configuration.workers"
              class="number-input"
              type="number"
              min="1"
              max="16"
              :disabled="controlsDisabled"
            /></label>
          <label class="field-label">ブラウザワーカー数<input
              v-model.number="configuration.browserWorkers"
              class="number-input"
              type="number"
              min="1"
              max="8"
              :disabled="controlsDisabled || metadataOnly"
            /></label>
          <label class="field-label">最大リクエスト / 秒<input
              v-model.number="configuration.maxRequestsPerSecond"
              class="number-input"
              type="number"
              min="1"
              max="100"
              :disabled="controlsDisabled"
            /></label>
        </div>
      </section>
      <section class="setting-section">
        <h2 class="section-heading" id="browser-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.browser"
            aria-controls="browser-settings"
            @click="toggleSection('browser')"
          >
            <span class="section-title">ブラウザ</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="browser-settings" v-show="sectionOpen.browser" class="section-content">
          <div class="field-grid">
            <label class="field-label">待機条件<select
                v-model="configuration.browserWait"
                class="select-input"
                :disabled="controlsDisabled || metadataOnly"
              >
                <option value="networkidle">通信完了まで</option>
                <option value="load">loadイベントまで</option>
                <option value="domcontentloaded">DOM構築まで</option>
              </select></label>
            <label class="field-label">タイムアウト（秒）<input
                v-model.number="configuration.browserTimeout"
                class="number-input"
                min="5"
                max="300"
                type="number"
                :disabled="controlsDisabled || metadataOnly"
              /></label>
          </div>
          <label class="field-label">ブラウザのパス<input
              v-model="configuration.browserPath"
              class="text-input"
              type="text"
              placeholder="自動検出"
              :disabled="controlsDisabled || metadataOnly"
            /></label>
          <label class="switch-row">
            <input
              v-model="configuration.autoDownloadBrowser"
              type="checkbox"
              :disabled="controlsDisabled || metadataOnly"
            />
            <span class="switch" aria-hidden="true"></span>
            <span>見つからない場合に自動取得</span>
          </label>
        </div>
      </section>
      <section class="setting-section output-section">
        <h2 class="section-heading" id="output-settings-heading">
          <button
            class="section-toggle"
            type="button"
            :aria-expanded="sectionOpen.output"
            aria-controls="output-settings"
            @click="toggleSection('output')"
          >
            <span class="section-title">保存先</span>
            <span class="section-chevron" aria-hidden="true"></span>
          </button>
        </h2>
        <div id="output-settings" v-show="sectionOpen.output" class="section-content">
          <div class="output-picker">
            <input
              v-model="configuration.outputRoot"
              class="text-input"
              type="text"
              aria-label="保存先フォルダ"
              :disabled="controlsDisabled"
            />
            <button
              type="button"
              :disabled="controlsDisabled || !isTauri"
              @click="chooseOutputFolder"
            >参照…</button>
          </div>
          <p class="path-note">実行ごとに日時フォルダ、サイズごとに専用フォルダを作成します。</p>
        </div>
      </section>
    </div>
    <div class="sidebar-footer">
      <button
        v-if="isBusy"
        class="primary-button stop-button"
        type="button"
        @click="cancelCrawl"
      ><AppIcon name="stop" /> クロールを中止</button>
      <button
        v-else
        class="primary-button"
        type="button"
        :disabled="!canStart"
        @click="beginCrawl"
      ><AppIcon name="play" /> {{ starting ? '準備しています…' : retryableRun ? 'もう一度実行' : 'クロールを開始' }}</button>
      <p v-if="!isTauri" class="run-note">ブラウザプレビュー・サンプルデータ。実行はデスクトップアプリで行います。</p>
      <p v-if="savingConfiguration" class="save-note" role="status">設定を保存しています…</p>
      <p v-if="storageError || saveError" class="save-note error" role="alert">{{ storageError || saveError }}</p>
      <div class="engine-line"><span class="engine-dot" :class="{ online: engineVersion !== '利用不可' }"></span> Engine: {{ engineVersion }}</div>
      <div class="license-line">SiteOne Crawler (MIT) · 非公式ラッパー</div>
    </div>
  </aside>
</template>

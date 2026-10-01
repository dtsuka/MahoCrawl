<script setup lang="ts">
import AppIcon from './AppIcon.vue'
import type { ScanRunSummary } from '../types'
import type { useHistory } from '../composables/useHistory'
import type { ComponentPublicInstance } from 'vue'

defineProps<{
  isBusy: boolean
  historyOpen: boolean
  historyRoot: string
  historyLoading: boolean
  historyOpeningPath: string
  historyError: string
  historyRuns: ScanRunSummary[]
  onHistoryModalKeydown: ReturnType<typeof useHistory>['onHistoryModalKeydown']
  closeHistory: ReturnType<typeof useHistory>['closeHistory']
  chooseScanFolder: ReturnType<typeof useHistory>['chooseScanFolder']
  refreshScanRuns: ReturnType<typeof useHistory>['refreshScanRuns']
  formatScanDate: ReturnType<typeof useHistory>['formatScanDate']
  openHistoricalRun: ReturnType<typeof useHistory>['openHistoricalRun']
  setHistoryModal: (element: Element | ComponentPublicInstance | null) => void
}>()
</script>

<template>
  <div
    v-if="historyOpen"
    class="preview-backdrop history-backdrop"
    role="presentation"
    @click.self="closeHistory"
  >
    <section
      :ref="setHistoryModal"
      class="history-modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="history-title"
      tabindex="-1"
      @keydown="onHistoryModalKeydown"
    >
      <header>
        <div>
          <h2 id="history-title">過去のスキャン</h2>
          <p>保存済みの結果を選ぶと、キャプチャとSEO情報を再表示します。</p>
        </div>
        <button type="button" aria-label="過去のスキャンを閉じる" @click="closeHistory">
          <AppIcon name="close" />
        </button>
      </header>
      <div class="history-toolbar">
        <div class="history-root">
          <span>参照中のフォルダ</span>
          <strong :title="historyRoot">{{ historyRoot }}</strong>
        </div>
        <div class="history-toolbar-actions">
          <button
            type="button"
            :disabled="historyLoading || !!historyOpeningPath"
            @click="chooseScanFolder"
          ><AppIcon name="folder" />別のフォルダ</button>
          <button
            type="button"
            :disabled="historyLoading || !!historyOpeningPath"
            aria-label="スキャン一覧を更新"
            @click="refreshScanRuns()"
          ><AppIcon name="refresh" />更新</button>
        </div>
      </div>
      <p v-if="isBusy" class="history-notice">クロール完了後に過去のスキャンを開けます。</p>
      <p v-if="historyError" class="history-error" role="alert">{{ historyError }}</p>
      <div class="history-list-wrap">
        <div v-if="historyLoading" class="history-state" role="status">
          <span class="spinner" aria-hidden="true"></span>
          <strong>スキャンを探しています</strong>
          <span>フォルダ内の実行結果を確認しています。</span>
        </div>
        <div v-else-if="historyRuns.length === 0" class="history-state">
          <AppIcon name="history" />
          <strong>スキャン結果が見つかりません</strong>
          <span>別の保存先を使っている場合は、フォルダを指定してください。</span>
          <button type="button" @click="chooseScanFolder">フォルダを指定</button>
        </div>
        <ul v-else class="history-list" aria-label="保存済みスキャン">
          <li v-for="run in historyRuns" :key="run.path">
            <div class="history-run-main">
              <strong :title="run.runId">{{ run.runId }}</strong>
              <span :title="run.path">{{ run.path }}</span>
            </div>
            <div class="history-run-meta">
              <time>{{ formatScanDate(run.modifiedAt) }}</time>
              <span>{{ run.sizeCount ? `${run.sizeCount}サイズ` : 'メタ情報のみ' }}</span>
              <span>{{ run.captureCount }}件のキャプチャ</span>
              <span v-if="run.hasHtmlReport">HTMLレポートあり</span>
            </div>
            <button
              type="button"
              :disabled="isBusy || (!!historyOpeningPath && historyOpeningPath !== run.path)"
              @click="openHistoricalRun(run)"
            >{{ historyOpeningPath === run.path ? '読み込み中…' : '開く' }}</button>
          </li>
        </ul>
      </div>
    </section>
  </div>
</template>

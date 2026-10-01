import type { CaptureItem } from '../types'

export type PreviewImageMode = 'fit' | 'actual' | 'width'

export type SettingsSection = 'target' | 'sizes' | 'capture' | 'crawl' | 'browser' | 'output'

export interface PreviewRequest {
  token: number
  path: string
  filename: string
  pageTitle: string
  pageUrl: string
  sizeId: string | null
}

export interface PreviewContext {
  pageTitle?: string
  pageUrl?: string
  sizeId?: string | null
  initialError?: string
}

export interface PreviewEntry {
  capture: CaptureItem
  context: PreviewContext
}

export interface RunSnapshot {
  runId: string
  root: string
  generation: number
}

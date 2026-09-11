import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { CaptureImage, CaptureItem, CrawlConfiguration, CrawlStatus, SeoPageItem, StartResponse } from './types'

export const isTauri = Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__)

export async function loadConfiguration(): Promise<CrawlConfiguration | null> {
  return invoke<CrawlConfiguration | null>('load_configuration')
}

export async function saveConfiguration(configuration: CrawlConfiguration): Promise<void> {
  await invoke('save_configuration', { configuration })
}

export async function validateConfigurationRust(configuration: CrawlConfiguration): Promise<void> {
  await invoke('validate_configuration', { configuration })
}

export async function getStatus(): Promise<CrawlStatus> {
  return invoke<CrawlStatus>('get_status')
}

export async function getLog(): Promise<string> {
  return invoke<string>('get_log')
}

export async function getEngineVersion(): Promise<string> {
  return invoke<string>('get_engine_version')
}

export async function startCrawl(configuration: CrawlConfiguration): Promise<StartResponse> {
  return invoke<StartResponse>('start_crawl', { configuration })
}

export async function stopCrawl(): Promise<void> {
  await invoke('stop_crawl')
}

export async function listCaptures(root: string, configuration: CrawlConfiguration): Promise<CaptureItem[]> {
  return invoke<CaptureItem[]>('list_captures', { root, configuration })
}

export async function listSeoPages(root: string): Promise<SeoPageItem[]> {
  return invoke<SeoPageItem[]>('list_seo_pages', { root })
}

export async function openPath(path: string): Promise<void> {
  await invoke('open_path', { path })
}

export async function revealPath(path: string): Promise<void> {
  await invoke('reveal_path', { path })
}

export async function readCapture(path: string): Promise<CaptureImage> {
  return invoke<CaptureImage>('read_capture', { path })
}

export async function readCaptureThumbnail(path: string): Promise<CaptureImage> {
  return invoke<CaptureImage>('read_capture_thumbnail', { path })
}

export async function selectOutputFolder(): Promise<string | null> {
  return invoke<string | null>('select_output_folder')
}

export async function clearLog(): Promise<void> {
  await invoke('clear_log')
}

export async function subscribeStatus(handler: (status: CrawlStatus) => void): Promise<UnlistenFn | null> {
  if (!isTauri) return null
  return listen<CrawlStatus>('crawl://status', (event) => handler(event.payload))
}

export async function subscribeOutput(handler: (text: string) => void): Promise<UnlistenFn | null> {
  if (!isTauri) return null
  return listen<{ text: string }>('crawl://output', (event) => handler(event.payload.text))
}

export async function subscribeCaptures(handler: (captures: CaptureItem[]) => void): Promise<UnlistenFn | null> {
  if (!isTauri) return null
  return listen<CaptureItem[]>('crawl://captures', (event) => handler(event.payload))
}

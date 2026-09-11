import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../src/App.vue'
import { cloneConfiguration, DEFAULT_CONFIGURATION, type CaptureItem, type CaptureViewport, type CrawlStatus, type SeoPageItem } from '../src/types'

const bridgeMock = vi.hoisted(() => ({
  clearLog: vi.fn(),
  getEngineVersion: vi.fn(),
  getLog: vi.fn(),
  getStatus: vi.fn(),
  isTauri: false as boolean,
  listCaptures: vi.fn(),
  listSeoPages: vi.fn(),
  loadConfiguration: vi.fn(),
  openPath: vi.fn(),
  readCapture: vi.fn(),
  readCaptureThumbnail: vi.fn(),
  revealPath: vi.fn(),
  saveConfiguration: vi.fn(),
  selectOutputFolder: vi.fn(),
  startCrawl: vi.fn(),
  stopCrawl: vi.fn(),
  subscribeCaptures: vi.fn(),
  subscribeOutput: vi.fn(),
  subscribeStatus: vi.fn(),
  validateConfigurationRust: vi.fn(),
}))

vi.mock('../src/bridge', () => bridgeMock)

const idleStatus: CrawlStatus = {
  phase: 'idle',
  runId: null,
  sizeIndex: 0,
  sizeTotal: 0,
  currentSizeId: null,
  currentSizeLabel: null,
  currentPlan: null,
  plans: [],
  runCaptures: [],
  message: null,
}

function prepareTauriMocks(): void {
  bridgeMock.isTauri = true
  bridgeMock.loadConfiguration.mockResolvedValue(cloneConfiguration(DEFAULT_CONFIGURATION))
  bridgeMock.getEngineVersion.mockResolvedValue('test-engine')
  bridgeMock.getLog.mockResolvedValue('')
  bridgeMock.getStatus.mockResolvedValue({ ...idleStatus })
  bridgeMock.listCaptures.mockResolvedValue([])
  bridgeMock.listSeoPages.mockResolvedValue([])
  bridgeMock.subscribeCaptures.mockResolvedValue(null)
  bridgeMock.subscribeOutput.mockResolvedValue(null)
  bridgeMock.subscribeStatus.mockResolvedValue(null)
}

async function mountApp(options: { tauri?: boolean } = {}) {
  if (options.tauri) prepareTauriMocks()
  const wrapper = mount(App, { attachTo: document.body })
  await nextTick()
  await nextTick()
  await flushPromises()
  return wrapper
}

describe('settings sidebar', () => {
  beforeEach(() => {
  vi.stubGlobal('localStorage', {
      getItem: vi.fn(() => null),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    clear: vi.fn(),
  })
  bridgeMock.isTauri = false
  bridgeMock.clearLog.mockReset()
  bridgeMock.getEngineVersion.mockReset()
  bridgeMock.getLog.mockReset()
  bridgeMock.getStatus.mockReset()
  bridgeMock.listCaptures.mockReset()
  bridgeMock.listSeoPages.mockReset()
  bridgeMock.loadConfiguration.mockReset()
  bridgeMock.openPath.mockReset()
  bridgeMock.readCapture.mockReset()
  bridgeMock.readCaptureThumbnail.mockReset()
  bridgeMock.revealPath.mockReset()
  bridgeMock.saveConfiguration.mockReset()
  bridgeMock.selectOutputFolder.mockReset()
  bridgeMock.startCrawl.mockReset()
  bridgeMock.stopCrawl.mockReset()
  bridgeMock.subscribeCaptures.mockReset()
  bridgeMock.subscribeOutput.mockReset()
  bridgeMock.subscribeStatus.mockReset()
  bridgeMock.validateConfigurationRust.mockReset()
})

  it('uses accessible toggles that update each section visibility', async () => {
    const wrapper = await mountApp()
    const toggles = wrapper.findAll<HTMLButtonElement>('button.section-toggle')

    expect(toggles).toHaveLength(6)
    for (const toggle of toggles) {
      const controls = toggle.attributes('aria-controls')
      expect(controls).toBeTruthy()
      expect(toggle.attributes('aria-expanded')).toMatch(/^(true|false)$/)
      expect(wrapper.find(`#${controls}`).exists()).toBe(true)
      expect(toggle.find('.section-chevron').attributes('aria-hidden')).toBe('true')
    }

    const sizesToggle = wrapper.find<HTMLButtonElement>('button[aria-controls="capture-size-settings"]')
    const sizesContent = wrapper.find('#capture-size-settings')
    expect(sizesToggle.attributes('aria-expanded')).toBe('true')
    expect(sizesContent.isVisible()).toBe(true)

    await sizesToggle.trigger('click')
    expect(sizesToggle.attributes('aria-expanded')).toBe('false')
    expect(sizesContent.attributes('style')).toContain('display: none')

    await sizesToggle.trigger('click')
    expect(sizesToggle.attributes('aria-expanded')).toBe('true')
    expect(sizesContent.attributes('style')).not.toContain('display: none')

    wrapper.unmount()
  })

  it('lets the target section collect optional Basic auth without exposing the password', async () => {
    const wrapper = await mountApp()
    const user = wrapper.find<HTMLInputElement>('#http-auth-user')
    const password = wrapper.find<HTMLInputElement>('#http-auth-password')

    expect(user.exists()).toBe(true)
    expect(password.exists()).toBe(true)
    expect(password.element.type).toBe('password')
    expect(wrapper.text()).toContain('パスワードは保存せず')

    await password.setValue('secret')
    expect(wrapper.text()).toContain('Basic認証のユーザー名を入力してください。')

    await user.setValue('staging')
    expect(wrapper.text()).not.toContain('Basic認証のユーザー名を入力してください。')
    expect(user.element.value).toBe('staging')
    expect(password.element.value).toBe('secret')

    wrapper.unmount()
  })

  it('keeps size fields directly editable and leaves only delete actions in each row', async () => {
    const wrapper = await mountApp()
    const rows = wrapper.findAll('.size-row')

    expect(rows).toHaveLength(3)
    expect(wrapper.findAll('button[aria-label$="を編集"]').length).toBe(0)
    expect(wrapper.findAll('.size-row.editing').length).toBe(0)
    expect(wrapper.findAll('button[aria-label$="を削除"]')).toHaveLength(3)
    expect(rows[0].find('.checkbox-label').exists()).toBe(true)
    expect(rows[0].find('.size-label-input').exists()).toBe(true)
    expect(rows[0].findAll('input.dimension-input')).toHaveLength(2)
    expect(rows[0].find('button[aria-label$="を削除"]').exists()).toBe(true)

    const label = rows[0].find<HTMLInputElement>('input.size-label-input')
    await label.setValue('Desktop Wide')
    expect(label.element.value).toBe('Desktop Wide')

    wrapper.unmount()
  })

  it('keeps blank dimensions invalid and labels every new or disabled control', async () => {
    const wrapper = await mountApp()
    const width = wrapper.find<HTMLInputElement>('.size-row input.dimension-input')
    await width.setValue('')
    expect(width.element.value).toBe('')
    expect(wrapper.text()).toContain('画面サイズは幅・高さとも320〜8192pxで指定してください。')
    await width.setValue('0')
    expect(width.element.value).toBe('0')
    expect(wrapper.text()).toContain('画面サイズは幅・高さとも320〜8192pxで指定してください。')

    const firstLabel = wrapper.find<HTMLInputElement>('.size-row input.size-label-input')
    await firstLabel.setValue('')
    expect(firstLabel.element.value).toBe('')

    expect(wrapper.find('input[aria-label="新しいサイズ名"]').exists()).toBe(true)
    const outputInput = wrapper.find<HTMLInputElement>('input[aria-label="保存先フォルダ"]')
    expect(outputInput.exists()).toBe(true)
    const cookieInput = wrapper.find<HTMLInputElement>('input[aria-describedby="cookie-banner-note"]')
    expect(cookieInput.element.disabled).toBe(true)
    expect(wrapper.text()).toContain('Cookieバナー自動非表示（現在未対応）')
    expect(wrapper.text()).toContain('現在のCLIでは自動非表示を実行しません')

    const newLabel = wrapper.find<HTMLInputElement>('input[aria-label="新しいサイズ名"]')
    const newWidth = wrapper.find<HTMLInputElement>('input[aria-label="新しいサイズの幅"]')
    await newLabel.setValue('空欄サイズ')
    await newWidth.setValue('')
    await wrapper.find('button.add-button').trigger('click')
    expect(wrapper.findAll('.size-row')).toHaveLength(3)
    expect(newWidth.element.value).toBe('')

    wrapper.unmount()
  })

  it('keeps the existing report and output actions without a dead more menu', async () => {
    const wrapper = await mountApp()
    const header = wrapper.find('header.run-header')

    expect(wrapper.find('button[aria-label="その他"]').exists()).toBe(false)
    expect(wrapper.find('.more-button').exists()).toBe(false)
    expect(header.findAll('.run-actions button')).toHaveLength(2)
    expect(header.text()).toContain('HTMLレポート')
    expect(header.text()).toContain('保存先')

    wrapper.unmount()
  })

  it('uses stable markup for the idle phase icon and toolbar controls', async () => {
    const wrapper = await mountApp()

    expect(wrapper.find('.phase-icon').attributes('aria-hidden')).toBe('true')
    expect(wrapper.find('.phase-icon .phase-ring').exists()).toBe(true)
    expect(wrapper.find('.phase-icon').text()).not.toContain('◌')
    expect(wrapper.find('select.sort-select').exists()).toBe(true)
    expect(wrapper.find('.path-note').exists()).toBe(true)

    wrapper.unmount()
  })

  it('switches between the default grid and an accessible SEO page table', async () => {
    const wrapper = await mountApp()
    const gridToggle = wrapper.find<HTMLButtonElement>('button[aria-label="グリッド表示"]')
    const listToggle = wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]')

    expect(gridToggle.attributes('aria-pressed')).toBe('true')
    expect(listToggle.attributes('aria-pressed')).toBe('false')
    expect(wrapper.find('.capture-grid').exists()).toBe(true)

    await listToggle.trigger('click')
    expect(gridToggle.attributes('aria-pressed')).toBe('false')
    expect(listToggle.attributes('aria-pressed')).toBe('true')
    expect(wrapper.find('table.results-table').exists()).toBe(true)
    expect(wrapper.findAll('table.results-table thead th')).toHaveLength(7)
    expect(wrapper.findAll('table.results-table tbody tr.result-row')).toHaveLength(2)
    expect(wrapper.text()).toContain('Example Domain')
    expect(wrapper.text()).toContain('未取得')

    const desktopCapture = wrapper.find<HTMLButtonElement>('button[aria-label="https://example.com/のDesktopキャプチャを表示"]')
    expect(desktopCapture.exists()).toBe(true)
    await desktopCapture.trigger('click')
    expect(wrapper.find('.preview-modal').exists()).toBe(true)
    expect(wrapper.find('.preview-modal h2').text()).toContain('Example Domain')
    expect(wrapper.find('.preview-modal').text()).toContain('https://example.com/')
    expect(wrapper.find('.preview-modal').text()).toContain('Desktop · 1440 × 900')
    expect(wrapper.find('.preview-modal .mock-page.large').exists()).toBe(true)

    await wrapper.find('button[aria-label="プレビューを閉じる"]').trigger('click')
    expect(wrapper.find('.preview-modal').exists()).toBe(false)

    await gridToggle.trigger('click')
    expect(gridToggle.attributes('aria-pressed')).toBe('true')
    expect(wrapper.find('.capture-grid').exists()).toBe(true)

    wrapper.unmount()
  })

  it('opens grid previews only from the article itself', async () => {
    const wrapper = await mountApp()
    const card = wrapper.find('.capture-card')
    const childButtons = card.findAll<HTMLButtonElement>('button')

    for (const button of childButtons) {
      await button.trigger('keydown', { key: 'Enter' })
      expect(wrapper.find('.preview-modal').exists()).toBe(false)
    }

    await card.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(wrapper.find('.preview-modal').exists()).toBe(true)

    await wrapper.find('button[aria-label="プレビューを閉じる"]').trigger('click')
    await card.trigger('keydown', { key: ' ' })
    await flushPromises()
    expect(wrapper.find('.preview-modal').exists()).toBe(true)

    wrapper.unmount()
  })

  it('resolves each row size by id even when labels are duplicated', async () => {
    const wrapper = await mountApp()
    await wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    const state = wrapper.vm as unknown as { seoPages: SeoPageItem[] }
    state.seoPages[0].sizeIds = ['desktop', 'tablet']
    state.seoPages[0].sizeLabels = ['同じ名前', '同じ名前']
    state.seoPages[0].captureBySize = {
      desktop: {
        path: '/captures/page-desktop.png', filename: 'page-desktop.png', bytes: 1200, modifiedAt: 1,
        sizeId: 'desktop', sizeLabel: '同じ名前', width: 1440, height: 900,
      },
      tablet: {
        path: '/captures/page-tablet.png', filename: 'page-tablet.png', bytes: 1300, modifiedAt: 2,
        sizeId: 'tablet', sizeLabel: '同じ名前', width: 768, height: 1024,
      },
    }
    await nextTick()

    const pageRow = wrapper.findAll('tbody tr.result-row').find((row) => row.find('.url-text').text() === 'https://example.com/')
    expect(pageRow).toBeTruthy()
    const sizeButtons = pageRow!.findAll<HTMLButtonElement>('button.size-button')
    expect(sizeButtons).toHaveLength(2)
    await sizeButtons[1].trigger('click')
    await flushPromises()
    expect(wrapper.find('.preview-modal').text()).toContain('page-tablet.png')
    expect(wrapper.find('.preview-modal').text()).toContain('同じ名前 · 768 × 1024')
    expect((wrapper.vm as unknown as { selectedCapture: CaptureItem }).selectedCapture.sizeId).toBe('tablet')

    wrapper.unmount()
  })

  it('shows a clear unavailable state and never guesses another capture', async () => {
    const wrapper = await mountApp()
    await wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    const state = wrapper.vm as unknown as { seoPages: SeoPageItem[]; captures: CaptureItem[] }
    state.seoPages[0].captureBySize = undefined
    state.captures[0].filename = 'example_com_01234567.png'
    await nextTick()

    await wrapper.find<HTMLButtonElement>('button[aria-label="https://example.com/のDesktopキャプチャを表示"]').trigger('click')
    expect(wrapper.find('.preview-error').text()).toContain('このページ・サイズのキャプチャは未取得です')
    expect(wrapper.find('.preview-modal').text()).toContain('キャプチャ未取得')
    expect(wrapper.find('.preview-modal').text()).toContain('https://example.com/')

    await wrapper.find('button[aria-label="プレビューを閉じる"]').trigger('click')
    expect(wrapper.find('.preview-modal').exists()).toBe(false)
    wrapper.unmount()
  })

  it('loads the mapped image through the Tauri bridge and reports bridge failures', async () => {
    const page: SeoPageItem = {
      url: 'https://example.com/contact', title: 'Contact', description: null, canonical: null,
      ogTitle: null, ogDescription: null, ogImage: null, twitterTitle: null, twitterDescription: null,
      twitterImage: null, h1: null, h2: null, sizeIds: ['desktop'], sizeLabels: ['Desktop'], status: 'Allowed',
      captureBySize: {
        desktop: {
          path: '/captures/contact-desktop.png', filename: 'contact-desktop.png', bytes: 2048, modifiedAt: 3,
          sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900,
        },
      },
    }
    const wrapper = await mountApp({ tauri: true })
    ;(wrapper.vm as unknown as { seoPages: SeoPageItem[] }).seoPages = [page]
    bridgeMock.readCapture.mockResolvedValue({ mimeType: 'image/png', dataBase64: 'ZmFrZQ==' })
    await wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    await wrapper.find<HTMLButtonElement>('button[aria-label="https://example.com/contactのDesktopキャプチャを表示"]').trigger('click')
    await flushPromises()

    expect(bridgeMock.readCapture).toHaveBeenCalledWith('/captures/contact-desktop.png')
    expect(wrapper.find('img[src="data:image/png;base64,ZmFrZQ=="]').exists()).toBe(true)
    expect(wrapper.find('.preview-modal').text()).toContain('contact-desktop.png')
    wrapper.unmount()

    const failureWrapper = await mountApp({ tauri: true })
    ;(failureWrapper.vm as unknown as { seoPages: SeoPageItem[] }).seoPages = [page]
    bridgeMock.readCapture.mockRejectedValue(new Error('bridge read failed'))
    await failureWrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    await failureWrapper.find<HTMLButtonElement>('button[aria-label="https://example.com/contactのDesktopキャプチャを表示"]').trigger('click')
    await flushPromises()

    expect(failureWrapper.find('.preview-error').text()).toContain('bridge read failed')
    failureWrapper.unmount()
  })

  it('keeps delayed reads from replacing a closed or reopened preview', async () => {
    const page: SeoPageItem = {
      url: 'https://example.com/race', title: 'Race', description: null, canonical: null,
      ogTitle: null, ogDescription: null, ogImage: null, twitterTitle: null, twitterDescription: null,
      twitterImage: null, h1: null, h2: null, sizeIds: ['desktop', 'tablet'], sizeLabels: ['Desktop', 'Tablet'], status: 'Allowed',
      captureBySize: {
        desktop: {
          path: '/captures/race-desktop.png', filename: 'race-desktop.png', bytes: 2048, modifiedAt: 3,
          sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900,
        },
        tablet: {
          path: '/captures/race-tablet.png', filename: 'race-tablet.png', bytes: 2048, modifiedAt: 3,
          sizeId: 'tablet', sizeLabel: 'Tablet', width: 768, height: 1024,
        },
      },
    }
    let resolveDesktop!: (value: { mimeType: string; dataBase64: string }) => void
    let resolveTablet!: (value: { mimeType: string; dataBase64: string }) => void
    bridgeMock.readCapture.mockImplementation((path: string) => new Promise((resolve) => {
      if (path.endsWith('desktop.png')) resolveDesktop = resolve
      else resolveTablet = resolve
    }))
    const wrapper = await mountApp({ tauri: true })
    ;(wrapper.vm as unknown as { seoPages: SeoPageItem[] }).seoPages = [page]
    await wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    const sizeButtons = wrapper.findAll<HTMLButtonElement>('button.size-button')
    await sizeButtons[0].trigger('click')
    await flushPromises()
    expect(wrapper.find('.preview-loading').text()).toContain('読み込み中')

    await wrapper.find('button[aria-label="プレビューを閉じる"]').trigger('click')
    await nextTick()
    expect(document.activeElement).toBe(sizeButtons[0].element)

    await sizeButtons[1].trigger('click')
    await flushPromises()
    resolveDesktop({ mimeType: 'image/png', dataBase64: 'c3RhbGU=' })
    await flushPromises()
    expect(wrapper.find('.preview-loading').exists()).toBe(true)
    expect(wrapper.find('img[src*="c3RhbGU="]').exists()).toBe(false)

    resolveTablet({ mimeType: 'image/png', dataBase64: 'Y3VycmVudA==' })
    await flushPromises()
    expect(wrapper.find('img[src="data:image/png;base64,Y3VycmVudA=="]').exists()).toBe(true)
    expect(wrapper.find('.preview-modal').text()).toContain('race-tablet.png')
    wrapper.unmount()
  })

  it('focuses the modal, traps Tab, blocks the crawl shortcut, and restores focus', async () => {
    const wrapper = await mountApp()
    await wrapper.find<HTMLButtonElement>('button[aria-label="行表示"]').trigger('click')
    const opener = wrapper.find<HTMLButtonElement>('button.size-button')
    await opener.trigger('click')
    await flushPromises()
    const modal = wrapper.find('.preview-modal')
    expect(document.activeElement).toBe(modal.element)

    const closeButton = wrapper.find<HTMLButtonElement>('button[aria-label="プレビューを閉じる"]')
    closeButton.element.focus()
    await modal.trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(closeButton.element)

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true }))
    expect(bridgeMock.startCrawl).not.toHaveBeenCalled()

    await modal.trigger('keydown', { key: 'Escape' })
    await nextTick()
    expect(wrapper.find('.preview-modal').exists()).toBe(false)
    expect(document.activeElement).toBe(opener.element)

    await opener.trigger('click')
    await flushPromises()
    const backdrop = wrapper.find('.preview-backdrop')
    await backdrop.trigger('click')
    expect(wrapper.find('.preview-modal').exists()).toBe(false)
    expect(document.activeElement).toBe(opener.element)
    wrapper.unmount()
  })

  it.each(['disabled', 'deleted'] as const)('starts a metadata-only crawl with %s sizes and displays its report', async (mode) => {
    prepareTauriMocks()
    let statusHandler: ((next: CrawlStatus) => void) | undefined
    bridgeMock.subscribeStatus.mockImplementation(async (handler: (next: CrawlStatus) => void) => {
      statusHandler = handler
      return vi.fn()
    })
    const root = '/metadata-run'
    bridgeMock.startCrawl.mockResolvedValue({ runId: 'metadata-run', root, totalSizes: 0 })
    const wrapper = await mountApp()
    if (mode === 'disabled') {
      for (const checkbox of wrapper.findAll('.size-row input[type="checkbox"]')) await checkbox.setValue(false)
    } else {
      for (const button of wrapper.findAll('.size-row button.delete')) await button.trigger('click')
      expect(wrapper.findAll('.size-row')).toHaveLength(0)
    }
    const startButton = wrapper.find<HTMLButtonElement>('.sidebar-footer .primary-button')
    expect(startButton.element.disabled).toBe(false)
    expect(wrapper.find('#capture-size-settings').text()).toContain('キャプチャなしでHTML内のメタ情報を取得します')
    expect(wrapper.find<HTMLSelectElement>('#capture-settings select').element.disabled).toBe(true)
    await startButton.trigger('click')
    await flushPromises()
    expect(bridgeMock.startCrawl).toHaveBeenCalledTimes(1)
    expect(bridgeMock.startCrawl.mock.calls[0][0].captures.filter((size: CaptureViewport) => size.enabled)).toEqual([])
    expect(wrapper.find('button[aria-label="行表示"]').attributes('aria-pressed')).toBe('true')
    expect(wrapper.find('.progress-label').text()).toBe('メタ情報を取得中（キャプチャなし）')
    expect(wrapper.find('[role="progressbar"]').attributes('aria-valuenow')).toBeUndefined()

    const plan = { root, sizeRoot: `${root}/metadata`, captures: `${root}/metadata/screenshots`, httpCacheDir: `${root}/metadata/.siteone-http-cache`, htmlReport: `${root}/metadata/report.html`, jsonReport: `${root}/metadata/report.json`, textReport: `${root}/metadata/report.txt`, sizeSlug: 'metadata' }
    bridgeMock.listSeoPages.mockResolvedValue([{
      url: 'https://example.com/', title: 'Metadata title', description: 'Metadata description', canonical: 'https://example.com/',
      ogTitle: null, ogDescription: null, ogImage: null, twitterTitle: null, twitterDescription: null, twitterImage: null,
      h1: 'Heading', h2: null, sizeIds: [], sizeLabels: [], captureBySize: {}, status: '200',
    }])
    statusHandler?.({ ...idleStatus, runId: 'metadata-run', phase: 'succeeded', plans: [plan], currentPlan: plan })
    await flushPromises()
    expect(wrapper.findAll('tr.result-row')).toHaveLength(1)
    expect(wrapper.find('tr.result-row').text()).toContain('Metadata title')
    expect(wrapper.find('tr.result-row').text()).toContain('キャプチャなし')
    expect(wrapper.findAll('.size-button')).toHaveLength(0)
    expect(wrapper.findAll('.filter-pills button')).toHaveLength(1)
    await wrapper.findAll('.run-actions button')[0].trigger('click')
    expect(bridgeMock.openPath).toHaveBeenCalledWith(plan.htmlReport)

    if (mode === 'disabled') {
      await wrapper.find('.size-row input[type="checkbox"]').setValue(true)
      expect(wrapper.findAll('.filter-pills button')).toHaveLength(1)
    } else {
      wrapper.unmount()
      bridgeMock.loadConfiguration.mockResolvedValue({ ...cloneConfiguration(DEFAULT_CONFIGURATION), captures: [] })
      const reloaded = await mountApp()
      expect(reloaded.findAll('.size-row')).toHaveLength(0)
      expect(reloaded.find<HTMLButtonElement>('.sidebar-footer .primary-button').element.disabled).toBe(false)
      reloaded.unmount()
      return
    }
    wrapper.unmount()
  })

  it('resets artifacts for a new run, ignores stale refreshes, and prevents double start', async () => {
    prepareTauriMocks()
    let statusHandler: ((next: CrawlStatus) => void) | undefined
    let captureHandler: ((next: CaptureItem[]) => void) | undefined
    bridgeMock.subscribeStatus.mockImplementation(async (handler: (next: CrawlStatus) => void) => {
      statusHandler = handler
      return vi.fn()
    })
    bridgeMock.subscribeCaptures.mockImplementation(async (handler: (next: CaptureItem[]) => void) => {
      captureHandler = handler
      return vi.fn()
    })
    bridgeMock.validateConfigurationRust.mockResolvedValue(undefined)
    let resolveStart!: (value: { runId: string; root: string; totalSizes: number }) => void
    bridgeMock.startCrawl.mockImplementation(() => new Promise((resolve) => { resolveStart = resolve }))
    const wrapper = await mountApp()
    const state = wrapper.vm as unknown as {
      captures: CaptureItem[]
      seoPages: SeoPageItem[]
      logText: string
      status: CrawlStatus
    }
    state.captures = [{ path: '/old/old.png', filename: 'old.png', bytes: 1, modifiedAt: 1, sizeId: 'desktop', sizeLabel: 'Desktop', width: 1, height: 1 }]
    state.seoPages = [{
      url: 'https://old.example/', title: 'Old', description: null, canonical: null,
      ogTitle: null, ogDescription: null, ogImage: null, twitterTitle: null, twitterDescription: null,
      twitterImage: null, h1: null, h2: null, sizeIds: ['desktop'], sizeLabels: ['Desktop'], status: 'Allowed',
    }]
    state.logText = 'old log'

    const startButton = wrapper.find<HTMLButtonElement>('.sidebar-footer .primary-button')
    await startButton.trigger('click')
    await nextTick()
    expect(startButton.element.disabled).toBe(true)
    await startButton.trigger('click')
    expect(bridgeMock.startCrawl).toHaveBeenCalledTimes(1)

    resolveStart({ runId: 'run-new', root: '/new', totalSizes: 2 })
    await flushPromises()
    expect(state.status.runId).toBe('run-new')
    expect(state.status.phase).toBe('running')
    expect(state.captures).toEqual([])
    expect(state.seoPages).toEqual([])
    expect(state.logText).toBe('')
    expect(captureHandler).toBeDefined()

    let resolveFirst!: (value: CaptureItem[]) => void
    let resolveSecond!: (value: CaptureItem[]) => void
    const first = new Promise<CaptureItem[]>((resolve) => { resolveFirst = resolve })
    const second = new Promise<CaptureItem[]>((resolve) => { resolveSecond = resolve })
    bridgeMock.listCaptures.mockImplementationOnce(() => first).mockImplementationOnce(() => second)
    const running = (message: string): CrawlStatus => ({
      ...state.status,
      phase: 'running',
      runId: 'run-new',
      currentPlan: {
        root: '/new', sizeRoot: '/new/Desktop', captures: '/new/Desktop/captures', httpCacheDir: '/new/cache',
        htmlReport: '/new/report.html', jsonReport: '/new/report.json', textReport: '/new/report.txt', sizeSlug: 'Desktop-1440x900',
      },
      message,
    })
    statusHandler?.(running('first'))
    statusHandler?.(running('second'))
    const latest = [{ path: '/new/latest.png', filename: 'latest.png', bytes: 2, modifiedAt: 2, sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900 }]
    const stale = [{ path: '/new/stale.png', filename: 'stale.png', bytes: 3, modifiedAt: 3, sizeId: 'desktop', sizeLabel: 'Desktop', width: 1440, height: 900 }]
    resolveSecond(latest)
    await flushPromises()
    resolveFirst(stale)
    await flushPromises()
    expect(state.captures).toEqual(latest)

    // The event payload is only a refresh hint; an empty stale event cannot
    // erase the known result while the guarded listing is pending.
    let resolveEventRefresh!: (value: CaptureItem[]) => void
    bridgeMock.listCaptures.mockImplementationOnce(() => new Promise((resolve) => { resolveEventRefresh = resolve }))
    captureHandler?.([])
    await nextTick()
    expect(state.captures).toEqual(latest)
    resolveEventRefresh(latest)
    await flushPromises()

    wrapper.unmount()
  })

  it('serializes native saves and lets the latest valid snapshot win', async () => {
    bridgeMock.validateConfigurationRust.mockResolvedValue(undefined)
    let resolveFirst!: () => void
    const firstSave = new Promise<void>((resolve) => { resolveFirst = resolve })
    bridgeMock.saveConfiguration.mockImplementationOnce(() => firstSave).mockResolvedValue(undefined)
    const wrapper = await mountApp({ tauri: true })
    const state = wrapper.vm as unknown as { configuration: { targetUrl: string } }
    expect(bridgeMock.saveConfiguration).not.toHaveBeenCalled()

    state.configuration.targetUrl = 'https://first.example/'
    await nextTick()
    await flushPromises()
    expect(bridgeMock.saveConfiguration).toHaveBeenCalledTimes(1)
    state.configuration.targetUrl = 'https://second.example/'
    await nextTick()
    resolveFirst()
    await flushPromises()
    await flushPromises()

    expect(bridgeMock.saveConfiguration).toHaveBeenCalledTimes(2)
    expect(bridgeMock.saveConfiguration.mock.calls[1][0].targetUrl).toBe('https://second.example/')
    wrapper.unmount()
  })
})

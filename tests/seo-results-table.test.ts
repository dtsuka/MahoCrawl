import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import SeoResultsTable from '../src/components/SeoResultsTable.vue'
import type { CaptureItem, CaptureViewport, SeoPageItem } from '../src/types'

function capture(sizeId: string, overrides: Partial<CaptureItem> = {}): CaptureItem {
  return {
    path: `/captures/page-${sizeId}.png`,
    filename: `page-${sizeId}.png`,
    bytes: 1200,
    modifiedAt: 10,
    sizeId,
    sizeLabel: null,
    width: 1440,
    height: 900,
    ...overrides,
  }
}

function page(overrides: Partial<SeoPageItem> = {}): SeoPageItem {
  return {
    url: 'https://example.test/long',
    title: '長いタイトル',
    description: '説明の一行目\n説明の二行目。これは詳細パネルで全文を確認できます。',
    canonical: 'https://example.test/long',
    ogTitle: 'OGPタイトル',
    ogDescription: 'OGPの詳細説明',
    ogImage: 'https://cdn.example.test/og.png',
    twitterTitle: 'Twitterタイトル',
    twitterDescription: 'Twitterの詳細説明',
    twitterImage: 'https://cdn.example.test/twitter.png',
    h1: '見出し1',
    h2: '見出し2',
    sizeIds: ['desktop', 'mobile'],
    sizeLabels: ['同名サイズ', '同名サイズ'],
    captureBySize: {
      desktop: capture('desktop', { sizeLabel: 'キャプチャDesktop' }),
      mobile: capture('mobile', { sizeLabel: 'キャプチャMobile', width: 390, height: 844 }),
    },
    status: 'Allowed',
    ...overrides,
  }
}

const sizes: CaptureViewport[] = [
  { id: 'desktop', label: '設定Desktop', width: 1440, height: 900, enabled: true },
  { id: 'mobile', label: '設定Mobile', width: 390, height: 844, enabled: true },
]

describe('SeoResultsTable', () => {
  it.each(['https://example.test/', ' HTTP://example.test/path '])('emits valid URL %s without opening captures or details', async (url) => {
    const wrapper = mount(SeoResultsTable, { props: { pages: [page({ url })], sizes } })
    const link = wrapper.find('button.url-text, a.url-text')
    expect(link.exists()).toBe(true)
    await link.trigger('click')
    expect(wrapper.emitted('open-url')?.[0]).toEqual([url.trim()])
    expect(wrapper.emitted('open-capture')).toBeUndefined()
    expect(wrapper.find('.detail-row').exists()).toBe(false)
    wrapper.unmount()
  })

  it.each(['', 'javascript:alert(1)', 'https://', 'https:///path', 'https://exa\n mple.test', '-https://example.test'])('keeps invalid URL %s as text', (url) => {
    const wrapper = mount(SeoResultsTable, { props: { pages: [page({ url })], sizes } })
    expect(wrapper.find('button.url-text, a.url-text').exists()).toBe(false)
    wrapper.unmount()
  })

  it('highlights only the active page and size', () => {
    const target = page()
    const wrapper = mount(SeoResultsTable, { props: { pages: [target], sizes, activePageUrl: target.url, activeSizeId: 'mobile' } })
    expect(wrapper.find('tr.result-row').classes()).toContain('is-active')
    expect(wrapper.findAll('.size-button.is-active')).toHaveLength(1)
    expect(wrapper.find('.size-button.is-active').text()).toBe('キャプチャMobile')
    wrapper.unmount()
  })

  it('keeps one main row, preserves size id order, and emits the clicked button as opener', async () => {
    const target = page()
    const wrapper = mount(SeoResultsTable, { props: { pages: [target], sizes } })

    expect(wrapper.findAll('tbody > tr.result-row')).toHaveLength(1)
    const buttons = wrapper.findAll<HTMLButtonElement>('button.size-button')
    expect(buttons).toHaveLength(2)
    expect(buttons.map((button) => button.text())).toEqual(['キャプチャDesktop', 'キャプチャMobile'])

    await buttons[1].trigger('click')
    const emitted = wrapper.emitted('open-capture')
    expect(emitted).toHaveLength(1)
    expect(emitted?.[0]).toEqual([target, 'mobile', buttons[1].element])

    wrapper.unmount()
  })

  it('expands a selectable full-text detail row and distinguishes missing values', async () => {
    const target = page({
      title: null,
      description: '長文の説明\n二行目も読める本文',
      canonical: null,
      ogDescription: null,
      twitterImage: null,
      h2: null,
    })
    const wrapper = mount(SeoResultsTable, { props: { pages: [target], sizes } })
    const toggle = wrapper.find<HTMLButtonElement>('button.detail-toggle')

    expect(toggle.attributes('aria-expanded')).toBe('false')
    expect(toggle.attributes('aria-controls')).toBe('seo-detail-0')
    expect(wrapper.findAll('tbody > tr.result-row')).toHaveLength(1)
    expect(wrapper.find('.detail-row').exists()).toBe(false)

    await toggle.trigger('click')
    expect(toggle.attributes('aria-expanded')).toBe('true')
    expect(wrapper.findAll('tbody > tr.result-row')).toHaveLength(1)
    expect(wrapper.find('.detail-row').exists()).toBe(true)
    expect(wrapper.find('.detail-panel').text()).toContain('長文の説明\n二行目も読める本文')
    expect(wrapper.find('.detail-panel').text()).toContain('Canonical未取得')
    expect(wrapper.find('.detail-panel').text()).toContain('twitter:image未取得')
    expect(wrapper.find('.detail-panel dd').attributes('class')).toBeDefined()

    await toggle.trigger('click')
    expect(toggle.attributes('aria-expanded')).toBe('false')
    expect(wrapper.find('.detail-row').exists()).toBe(false)
    wrapper.unmount()
  })

  it('groups detail entries into labeled sections with one entry per row', async () => {
    const wrapper = mount(SeoResultsTable, { props: { pages: [page({ canonical: null })], sizes } })
    await wrapper.find('button.detail-toggle').trigger('click')

    const groups = wrapper.findAll('.detail-group')
    expect(groups.map((group) => group.find('h4').text())).toEqual(['基本情報', 'OGP', 'Twitter', '見出し'])
    expect(groups.map((group) => group.findAll('.detail-entry dt').map((dt) => dt.text()))).toEqual([
      ['URL', 'Title', 'Description', 'Canonical'],
      ['og:title', 'og:description', 'og:image'],
      ['twitter:title', 'twitter:description', 'twitter:image'],
      ['H1', 'H2'],
    ])
    // 各グループは見出しで名前付けされた dl として読み上げられる
    for (const group of groups) {
      const headingId = group.find('h4').attributes('id')
      expect(headingId).toBeTruthy()
      expect(group.find('dl').attributes('aria-labelledby')).toBe(headingId)
    }
    const canonical = groups[0]!.findAll('.detail-entry')[3]!
    expect(canonical.classes()).toContain('is-missing')
    expect(canonical.find('dd').classes()).toContain('missing')
    wrapper.unmount()
  })
})

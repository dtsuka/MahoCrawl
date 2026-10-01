import { describe, expect, it } from 'vitest'
import { pageSizeOptions, searchCaptures, searchSeoPages, sortSeoPages } from '../src/gallery'
import type { CaptureItem, SeoPageItem } from '../src/types'

function capture(overrides: Partial<CaptureItem> = {}): CaptureItem {
  return {
    path: '',
    filename: 'home-desktop.png',
    bytes: 100,
    modifiedAt: 1,
    sizeId: 'desktop',
    sizeLabel: 'Desktop',
    width: 1440,
    height: 900,
    ...overrides,
  }
}

function page(overrides: Partial<SeoPageItem> = {}): SeoPageItem {
  return {
    url: 'https://example.test/',
    title: 'ホーム',
    description: 'トップページ',
    canonical: null,
    ogTitle: null,
    ogDescription: null,
    ogImage: null,
    twitterTitle: null,
    twitterDescription: null,
    twitterImage: null,
    h1: null,
    h2: null,
    sizeIds: ['desktop'],
    sizeLabels: ['Desktop'],
    captureBySize: {
      desktop: capture(),
    },
    status: 'Allowed',
    ...overrides,
  }
}

describe('gallery search and sort helpers', () => {
  it('keeps empty searches in source order and does not mutate arrays', () => {
    const items = [capture({ filename: 'z.png' }), capture({ filename: 'a.png' })]
    const pages = [page({ url: 'https://example.test/z' }), page({ url: 'https://example.test/a' })]
    const originalItems = [...items]
    const originalPages = [...pages]

    expect(searchCaptures(items, pages, '   ')).toEqual(items)
    expect(searchSeoPages(pages, '　')).toEqual(pages)
    expect(searchCaptures(items, pages, '   ')).not.toBe(items)
    expect(searchSeoPages(pages, '')).not.toBe(pages)
    expect(items).toEqual(originalItems)
    expect(pages).toEqual(originalPages)
  })

  it('matches Japanese, full-width/case-insensitive text, and all whitespace terms', () => {
    const items = [capture({ filename: 'HOME-WIDE.PNG', sizeLabel: 'デスクトップ' }), capture({ filename: 'contact.png' })]
    const pages = [page({ title: '会社案内', description: 'お問い合わせフォーム' })]

    expect(searchCaptures(items, pages, 'ｈｏｍｅ　デスクトップ')).toEqual([items[0]])
    expect(searchSeoPages(pages, '会社 お問い合わせ')).toEqual([pages[0]])
    expect(searchSeoPages(pages, '会社 存在しない')).toEqual([])
  })

  it('uses only explicit capture mappings, including pathless demo keys and distinct size ids', () => {
    const desktop = capture({ filename: 'same.png', sizeId: 'desktop', sizeLabel: '同じ名前' })
    const mobile = capture({ filename: 'same.png', sizeId: 'mobile', sizeLabel: '同じ名前' })
    const mappedPage = page({
      url: 'https://example.test/mapped',
      title: '明示ページ',
      captureBySize: { desktop, mobile },
    })
    const unrelatedPage = page({
      url: 'https://example.test/inferred',
      title: '推測ページ',
      captureBySize: undefined,
    })
    const unknown = capture({ filename: 'same.png', sizeId: 'tablet' })

    expect(searchCaptures([desktop, mobile], [mappedPage, unrelatedPage], '明示ページ')).toEqual([desktop, mobile])
    expect(searchCaptures([unknown], [mappedPage, unrelatedPage], '明示ページ')).toEqual([])
    expect(searchCaptures([unknown], [mappedPage, unrelatedPage], '推測ページ')).toEqual([])
  })

  it('sorts title fallbacks and newest timestamps deterministically while preserving input', () => {
    const missingTitle = page({ url: 'https://example.test/z', title: null, captureBySize: undefined })
    const titled = page({ url: 'https://example.test/b', title: 'アルファ', captureBySize: { desktop: capture({ modifiedAt: 20 }) } })
    const newest = page({ url: 'https://example.test/a', title: 'ベータ', captureBySize: { desktop: capture({ modifiedAt: 20 }), mobile: capture({ modifiedAt: 30, sizeId: 'mobile' }) } })
    const pages = [missingTitle, titled, newest]
    const original = [...pages]

    expect(sortSeoPages(pages, 'title')).toEqual([missingTitle, titled, newest])
    expect(sortSeoPages(pages, 'newest')).toEqual([newest, titled, missingTitle])
    expect(sortSeoPages(pages, 'url')).toEqual([newest, titled, missingTitle])
    expect(pages).toEqual(original)
  })
})


describe('page size options', () => {
  it('uses mapped labels, configured labels, aligned report labels, then ids in order', () => {
    const item = page({
      sizeIds: ['mapped', 'configured', 'reported', 'unknown'],
      sizeLabels: ['ignored', 'ignored', ' Report ', ' '],
      captureBySize: { mapped: capture({ sizeLabel: ' Mapped ' }) },
    })
    const sizes = [{ id: 'configured', label: ' Configured ', width: 390, height: 844, enabled: true }]
    expect(pageSizeOptions(item, sizes)).toEqual([
      { id: 'mapped', label: 'Mapped' },
      { id: 'configured', label: 'Configured' },
      { id: 'reported', label: 'Report' },
      { id: 'unknown', label: 'unknown' },
    ])
  })

  it('does not guess labels from misaligned arrays or alter the inputs', () => {
    const item = page({ sizeIds: ['first', 'second'], sizeLabels: ['wrong'], captureBySize: undefined })
    const original = structuredClone(item)
    expect(pageSizeOptions(item, [])).toEqual([{ id: 'first', label: 'first' }, { id: 'second', label: 'second' }])
    expect(item).toEqual(original)
    expect(pageSizeOptions(page({ sizeIds: [] }), [])).toEqual([])
  })
})

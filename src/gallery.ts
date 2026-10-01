import type { CaptureItem, CaptureViewport, SeoPageItem } from './types'

/**
 * Fields that are useful when looking for a page in either gallery view.
 *
 * Keep this list explicit: a capture is associated with a page only when the
 * page's `captureBySize` map says so.  In particular, do not infer a page
 * from a filename or URL slug.
 */
function pageSearchText(page: SeoPageItem, includeSizeLabels = false): string {
  const fields: Array<string | null | undefined> = [
    page.url,
    page.title,
    page.description,
    page.canonical,
    page.ogTitle,
    page.ogDescription,
    page.ogImage,
    page.twitterTitle,
    page.twitterDescription,
    page.twitterImage,
    page.h1,
    page.h2,
  ]
  if (includeSizeLabels) fields.push(...page.sizeLabels)
  return fields.filter((field): field is string => typeof field === 'string' && field.length > 0).join('\u0000')
}

/**
 * Normalise user-visible text for matching. NFKC makes full-width Latin
 * characters and spaces behave like their ordinary equivalents while
 * preserving Japanese text. The query itself is never treated as a regex.
 */
function normalizeSearchText(value: string): string {
  return value.normalize('NFKC').toLocaleLowerCase('en-US')
}

function queryTerms(query: string): string[] {
  const normalized = normalizeSearchText(query).trim()
  return normalized ? normalized.split(/\s+/u).filter(Boolean) : []
}

function matchesAllTerms(text: string, terms: string[]): boolean {
  if (terms.length === 0) return true
  const normalizedText = normalizeSearchText(text)
  return terms.every((term) => normalizedText.includes(term))
}

/**
 * Return a stable key for a capture that appears in an explicit
 * `captureBySize` entry. Real captures use their path. Browser/demo captures
 * can intentionally have no path, so those use filename + size id instead.
 */
export function captureIdentity(capture: CaptureItem, sizeIdHint?: string): string | null {
  if (capture.path.length > 0) return `path\u0000${capture.path}`

  const filename = capture.filename
  const sizeId = sizeIdHint || capture.sizeId || ''
  if (!filename || !sizeId) return null
  return `filename\u0000${filename}\u0000${sizeId}`
}

/** 明示された対応表を検索とプレビューで共用する。 */
export function buildCapturePageIndex(pages: SeoPageItem[]): Map<string, SeoPageItem[]> {
  const index = new Map<string, SeoPageItem[]>()
  for (const page of pages) {
    for (const [sizeId, capture] of Object.entries(page.captureBySize ?? {})) {
      if (!capture) continue
      const key = captureIdentity(capture, sizeId)
      if (!key) continue
      const mapped = index.get(key) ?? []
      if (!mapped.includes(page)) mapped.push(page)
      index.set(key, mapped)
    }
  }
  return index
}

/** 対応が一意なキャプチャのページだけを返す。 */
export function pageForCapture(capture: CaptureItem, index: Map<string, SeoPageItem[]>): SeoPageItem | null {
  const key = captureIdentity(capture)
  const pages = key ? index.get(key) : undefined
  return pages?.length === 1 ? pages[0]! : null
}

/**
 * Search captures by their own filename/size label and metadata from pages
 * explicitly mapped through `captureBySize`.
 */
export function searchCaptures(items: CaptureItem[], pages: SeoPageItem[], query: string): CaptureItem[] {
  const terms = queryTerms(query)
  if (terms.length === 0) return [...items]

  const pageIndex = buildCapturePageIndex(pages)
  return items.filter((capture) => {
    const key = captureIdentity(capture)
    const mappedPage = key ? (pageIndex.get(key) ?? []).map((page) => pageSearchText(page)).join('\u0000') : ''
    const captureText = [capture.filename, capture.sizeLabel, mappedPage]
      .filter((field): field is string => typeof field === 'string' && field.length > 0)
      .join('\u0000')
    return matchesAllTerms(captureText, terms)
  })
}

/** Search SEO pages by page metadata and configured capture size labels. */
export function searchSeoPages(pages: SeoPageItem[], query: string): SeoPageItem[] {
  const terms = queryTerms(query)
  if (terms.length === 0) return [...pages]

  return pages.filter((page) => matchesAllTerms(pageSearchText(page, true), terms))
}

export type SeoPageSortOrder = 'url' | 'title' | 'newest'

function sortText(value: string): string {
  return normalizeSearchText(value.trim())
}

function compareText(left: string, right: string): number {
  const normalizedLeft = sortText(left)
  const normalizedRight = sortText(right)
  return normalizedLeft.localeCompare(normalizedRight, 'ja', { numeric: true, sensitivity: 'base' })
    || left.localeCompare(right, 'ja', { numeric: true })
}

function titleSortValue(page: SeoPageItem): string {
  return page.title?.trim() || page.url
}

function latestModifiedAt(page: SeoPageItem): number | null {
  let latest: number | null = null
  for (const capture of Object.values(page.captureBySize ?? {})) {
    if (!capture || !Number.isFinite(capture.modifiedAt)) continue
    if (latest === null || capture.modifiedAt > latest) latest = capture.modifiedAt
  }
  return latest
}

/**
 * Sort SEO pages without mutating the source array.
 *
 * Missing titles consistently fall back to the page URL. For newest order,
 * pages with no valid mapped capture timestamp are placed after dated pages;
 * URL is the deterministic tie-breaker in every order.
 */
export function sortSeoPages(pages: SeoPageItem[], order: SeoPageSortOrder): SeoPageItem[] {
  return pages
    .map((page, index) => ({ page, index, modifiedAt: latestModifiedAt(page) }))
    .sort((left, right) => {
      let comparison = 0
      if (order === 'newest') {
        if (left.modifiedAt === null && right.modifiedAt !== null) return 1
        if (left.modifiedAt !== null && right.modifiedAt === null) return -1
        if (left.modifiedAt !== null && right.modifiedAt !== null) {
          comparison = right.modifiedAt - left.modifiedAt
        }
      } else if (order === 'title') {
        comparison = compareText(titleSortValue(left.page), titleSortValue(right.page))
      } else {
        comparison = compareText(left.page.url, right.page.url)
      }

      if (comparison !== 0) return comparison
      if (order === 'title' || order === 'newest') comparison = compareText(left.page.url, right.page.url)
      if (comparison !== 0) return comparison
      return left.index - right.index
    })
    .map(({ page }) => page)
}


/** Prefer the explicit capture mapping over configuration and aligned report labels. */
export function pageSizeOptions(page: SeoPageItem, sizes: CaptureViewport[]): Array<{ id: string; label: string }> {
  return page.sizeIds.map((id, index) => ({
    id,
    label: page.captureBySize?.[id]?.sizeLabel?.trim()
      || sizes.find((size) => size.id === id)?.label?.trim()
      || (page.sizeIds.length === page.sizeLabels.length ? page.sizeLabels[index]?.trim() : '')
      || id,
  }))
}

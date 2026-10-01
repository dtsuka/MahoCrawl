/** 外部ブラウザに渡せるHTTP(S) URLだけを返す。 */
export function externalUrl(value: string): string | null {
  if (/[\u0000-\u001f\u007f-\u009f]/u.test(value)) return null
  const trimmed = value.trim()
  if (!/^https?:\/\/[^/\\?#\s]+/iu.test(trimmed)) return null
  try {
    const url = new URL(trimmed)
    return (url.protocol === 'http:' || url.protocol === 'https:') && url.hostname ? trimmed : null
  } catch {
    return null
  }
}

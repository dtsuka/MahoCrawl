import { afterEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useOutputBuffer } from '../src/composables/useOutputBuffer'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('output buffer', () => {
  it('batches visible chunks into one animation frame', () => {
    const frames: FrameRequestCallback[] = []
    const raf = vi.fn((callback: FrameRequestCallback) => { frames.push(callback); return frames.length })
    vi.stubGlobal('requestAnimationFrame', raf)
    const buffer = useOutputBuffer(ref(true))
    buffer.append('first')
    buffer.append('second')
    expect(buffer.logText.value).toBe('')
    expect(raf).toHaveBeenCalledTimes(1)
    frames[0](0)
    expect(buffer.logText.value).toBe('firstsecond')
    buffer.dispose()
  })

  it('does no scheduling or reactive text updates while hidden, retaining only the tail', () => {
    const raf = vi.fn(() => 1)
    vi.stubGlobal('requestAnimationFrame', raf)
    const visible = ref(false)
    const buffer = useOutputBuffer(visible)
    buffer.append('old'.repeat(200_000))
    buffer.append('tail')
    expect(buffer.logText.value).toBe('')
    expect(raf).not.toHaveBeenCalled()
    visible.value = true
    expect(buffer.logText.value).toBe(('old'.repeat(200_000) + 'tail').slice(-400_000))
    buffer.dispose()
  })

  it('falls back without animation frames and discards queued chunks on clear and dispose', () => {
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', undefined)
    const buffer = useOutputBuffer(ref(true))
    buffer.append('old')
    buffer.clear()
    buffer.append('new')
    vi.runAllTimers()
    expect(buffer.logText.value).toBe('new')
    buffer.replace('x'.repeat(400_001))
    expect(buffer.logText.value).toHaveLength(400_000)
    buffer.append('discard')
    buffer.dispose()
    vi.runAllTimers()
    expect(buffer.logText.value).toBe('x'.repeat(400_000))
  })

  it('cancels a pending frame when hidden and flushes buffered output on return', () => {
    const frames: FrameRequestCallback[] = []
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return 7 })
    const cancel = vi.fn()
    vi.stubGlobal('cancelAnimationFrame', cancel)
    const visible = ref(true)
    const buffer = useOutputBuffer(visible)
    buffer.append('before')
    visible.value = false
    expect(cancel).toHaveBeenCalledWith(7)
    frames[0](0)
    buffer.append('after')
    expect(buffer.logText.value).toBe('')
    visible.value = true
    expect(buffer.logText.value).toBe('beforeafter')
    buffer.dispose()
  })
})

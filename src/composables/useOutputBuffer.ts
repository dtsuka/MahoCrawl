import { ref, watch, type Ref } from 'vue'

const MAX_LOG_LENGTH = 400_000

/** Keep event handling non-reactive; join the bounded chunks only when visible. */
export function useOutputBuffer(visible: Readonly<Ref<boolean>>) {
  const logText = ref('')
  let chunks: string[] = []
  let head = 0
  let length = 0
  let dirty = false
  let disposed = false
  let cancelScheduled: (() => void) | null = null
  let frameGeneration = 0

  function cancel(): void {
    frameGeneration += 1
    cancelScheduled?.()
    cancelScheduled = null
  }

  function flush(): void {
    if (disposed || !visible.value || !dirty) return
    chunks = chunks.slice(head)
    head = 0
    logText.value = chunks.join('')
    dirty = false
  }

  function schedule(): void {
    if (disposed || !visible.value || cancelScheduled) return
    const generation = ++frameGeneration
    const callback = () => {
      if (generation !== frameGeneration || disposed) return
      cancelScheduled = null
      flush()
    }
    if (typeof requestAnimationFrame === 'function') {
      const id = requestAnimationFrame(callback)
      cancelScheduled = () => {
        if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(id)
      }
    } else {
      const id = setTimeout(callback, 0)
      cancelScheduled = () => clearTimeout(id)
    }
  }

  function append(text: string): void {
    if (disposed || !text) return
    // A single oversized event can replace the buffer without retaining its prefix.
    if (text.length >= MAX_LOG_LENGTH) {
      chunks = [text.slice(-MAX_LOG_LENGTH)]
      head = 0
      length = MAX_LOG_LENGTH
    } else {
      chunks.push(text)
      length += text.length
      while (length > MAX_LOG_LENGTH) {
        const excess = length - MAX_LOG_LENGTH
        const first = chunks[head]!
        if (first.length <= excess) {
          length -= first.length
          head += 1
        } else {
          chunks[head] = first.slice(excess)
          length -= excess
        }
      }
      if (head > 1024 && head > chunks.length / 2) {
        chunks = chunks.slice(head)
        head = 0
      }
    }
    dirty = true
    schedule()
  }

  function clear(): void {
    cancel()
    chunks = []
    head = 0
    length = 0
    dirty = false
    logText.value = ''
  }

  function replace(text: string): void {
    clear()
    append(text)
    cancel()
    flush()
  }

  const stopWatching = watch(visible, (shown) => {
    cancel()
    if (shown) flush()
  }, { flush: 'sync' })

  function dispose(): void {
    disposed = true
    cancel()
    stopWatching()
    chunks = []
  }

  return { logText, append, clear, replace, dispose }
}

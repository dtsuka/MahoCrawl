import { beforeEach, expect, it, vi } from 'vitest'
import { listCaptures } from '../src/bridge'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

beforeEach(() => invoke.mockReset())

it('lists captures with only the root argument and forwards the result', async () => {
  const captures = [{ path: '/run/capture.png' }]
  invoke.mockResolvedValue(captures)
  expect(listCaptures).toHaveLength(1)
  expect(await listCaptures('/run')).toBe(captures)
  expect(invoke).toHaveBeenCalledWith('list_captures', { root: '/run' })
})

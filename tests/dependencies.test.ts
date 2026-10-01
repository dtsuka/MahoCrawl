import { readFileSync } from 'node:fs'
import { expect, it } from 'vitest'

it('omits the unused dialog plugin from the frontend manifest and lockfile', () => {
  const manifest = JSON.parse(readFileSync('package.json', 'utf8'))
  const lock = JSON.parse(readFileSync('package-lock.json', 'utf8'))
  expect(manifest.dependencies).not.toHaveProperty('@tauri-apps/plugin-dialog')
  expect(lock.packages[''].dependencies).not.toHaveProperty('@tauri-apps/plugin-dialog')
  expect(lock.packages).not.toHaveProperty('node_modules/@tauri-apps/plugin-dialog')
})

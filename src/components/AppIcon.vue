<script setup lang="ts">
const paths = {
  globe: 'M14 8A6 6 0 1 1 2 8a6 6 0 0 1 12 0ZM2 8h12M8 2c-3 3-3 9 0 12M8 2c3 3 3 9 0 12',
  grid: 'M2 2h4.5v4.5H2zM9.5 2H14v4.5H9.5zM2 9.5h4.5V14H2zM9.5 9.5H14V14H9.5z',
  list: 'M2 3h1M6 3h8M2 8h1M6 8h8M2 13h1M6 13h8',
  close: 'm4 4 8 8M12 4l-8 8',
  external: 'M9 2h5v5M14 2 7 9M6 3H2v11h11v-4',
  folder: 'M2 4h4l2 2h6v7H2zM2 4V2h4l2 2',
  history: 'M3 4V1M3 1H0M3 1 1.3 2.7M3 4a6 6 0 1 1-.1 8M8 4.5V8l2.5 1.5',
  refresh: 'M13 5V2m0 0h-3m3 0-2 2M3 11v3m0 0h3m-3 0 2-2M12.5 6A5 5 0 0 0 4 4M3.5 10A5 5 0 0 0 12 12',
  play: 'm5 2 9 6-9 6z',
  stop: 'M3 3h10v10H3z',
  check: 'm3 8 3 3 7-7',
  alert: 'M8 3v6M8 12v.5',
  image: 'M2 2h12v12H2zM4 11l3-3 2 2 2-2 3 3M5 5h.01',
  desktop: 'M2 2h12v9H2zM8 11v3M5 14h6',
  mobile: 'M5 1.5h6v13H5zM7.5 12h1',
  maximize: 'M6 2H2v4M10 2h4v4M2 10v4h4M14 10v4h-4',
  restore: 'M2 6h4V2M14 6h-4V2M6 14v-4H2M10 14v-4h4',
  previous: 'm10 3-5 5 5 5',
  next: 'm6 3 5 5-5 5',
  plus: 'M8 3v10M3 8h10',
} as const

defineProps<{ name: keyof typeof paths }>()
</script>

<template>
  <svg class="app-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
    <path :d="paths[name]" />
  </svg>
</template>

<script setup lang="ts">
const paths = {
  grid: 'M2 2h4.5v4.5H2zM9.5 2H14v4.5H9.5zM2 9.5h4.5V14H2zM9.5 9.5H14V14H9.5z',
  list: 'M2 3h1M6 3h8M2 8h1M6 8h8M2 13h1M6 13h8',
  close: 'm4 4 8 8M12 4l-8 8',
  external: 'M9 2h5v5M14 2 7 9M6 3H2v11h11v-4',
  folder: 'M2 4h4l2 2h6v7H2zM2 4V2h4l2 2',
  play: 'm5 2 9 6-9 6z',
  stop: 'M3 3h10v10H3z',
  check: 'm3 8 3 3 7-7',
  alert: 'M8 3v6M8 12v.5',
  image: 'M2 2h12v12H2zM4 11l3-3 2 2 2-2 3 3M5 5h.01',
  desktop: 'M2 2h12v9H2zM8 11v3M5 14h6',
  mobile: 'M5 1.5h6v13H5zM7.5 12h1',
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

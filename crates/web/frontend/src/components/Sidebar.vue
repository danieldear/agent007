<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

defineProps({ current: String, connected: Boolean })
defineEmits(['navigate'])

const { api } = useApi()
const projectName = ref('')
const projectPath = ref('')

const DARK_THEMES  = ['night', 'forest', 'ocean', 'aurora']
const DARK_THEME   = 'night'
const LIGHT_THEME  = 'day'

function normalizeSaved(t) {
  if (!t || t === 'dark')  return DARK_THEME
  if (t === 'light')       return LIGHT_THEME
  return t
}

const theme = ref(normalizeSaved(localStorage.getItem('theme')) || DARK_THEME)

function applyTheme(t) {
  document.documentElement.setAttribute('data-theme', t)
  localStorage.setItem('theme', t)
  theme.value = t
}

function toggleTheme() {
  applyTheme(DARK_THEMES.includes(theme.value) ? LIGHT_THEME : DARK_THEME)
}

onMounted(() => {
  applyTheme(normalizeSaved(localStorage.getItem('theme')))
  api.getStats().then(s => {
    if (s?.project_name) projectName.value = s.project_name
    if (s?.project_path) projectPath.value = s.project_path
  }).catch(() => {})
})

const primaryNav = [
  { id: 'dashboard', label: 'Tasks',     symbol: '▶' },
  { id: 'agents',    label: 'Personas',  symbol: '◉' },
  { id: 'skills',    label: 'Skills',    symbol: '⚡' },
  { id: 'workflows', label: 'Workflows', symbol: '⬡' },
  { id: 'memory',    label: 'Memory',    symbol: '◈' },
]

const configNav = [
  { id: 'mcp',        label: 'MCP',        symbol: '⬢' },
  { id: 'tools',      label: 'Tools',      symbol: '⊕' },
  { id: 'extensions', label: 'Extensions', symbol: '⊞' },
  { id: 'lsp',        label: 'LSP',        symbol: '⌘' },
  { id: 'sharing',    label: 'Sharing',    symbol: '⇅' },
  { id: 'help',       label: 'Guide',      symbol: '?' },
]

const isDark = () => DARK_THEMES.includes(theme.value)
</script>

<template>
  <aside class="w-48 bg-base-200 border-r border-base-300/80 flex flex-col shrink-0">

    <!-- Branding -->
    <div class="px-4 pt-5 pb-4 border-b border-base-300/80">
      <div class="flex items-baseline gap-2">
        <h1 class="text-base font-bold font-mono text-primary tracking-widest uppercase">agent007</h1>
        <span class="text-[10px] font-mono text-base-content/30 tracking-wider">v0.1</span>
      </div>
      <p class="text-[11px] text-base-content/40 mt-0.5 tracking-wide">AI Orchestration</p>
      <div
        v-if="projectName"
        class="mt-3 px-2 py-1 rounded border border-primary/20 bg-primary/5 flex items-center gap-1.5"
        :title="projectPath"
      >
        <span class="text-[9px] font-mono text-primary/60 uppercase tracking-widest">proj</span>
        <span class="text-[11px] font-mono text-primary/80 truncate">{{ projectName }}</span>
      </div>
    </div>

    <!-- Primary nav -->
    <nav class="py-2 space-y-0.5">
      <button
        v-for="item in primaryNav"
        :key="item.id"
        class="relative w-full text-left px-4 py-2.5 flex items-center gap-3 transition-colors duration-100"
        :class="current === item.id
          ? 'text-primary bg-primary/8 before:absolute before:left-0 before:top-0 before:bottom-0 before:w-0.5 before:bg-primary before:rounded-r'
          : 'text-base-content/55 hover:text-base-content/90 hover:bg-base-300/40'"
        @click="$emit('navigate', item.id)"
      >
        <span
          class="text-sm w-4 text-center shrink-0"
          :class="current === item.id ? 'text-primary' : 'text-base-content/30'"
        >{{ item.symbol }}</span>
        <span class="font-mono text-[12px] tracking-wide">{{ item.label }}</span>
      </button>
    </nav>

    <!-- Config section -->
    <div class="px-4 pt-3 pb-1">
      <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/25 mb-1.5">Config</div>
    </div>
    <nav class="pb-2 space-y-0.5">
      <button
        v-for="item in configNav"
        :key="item.id"
        class="relative w-full text-left px-4 py-2 flex items-center gap-3 transition-colors duration-100"
        :class="current === item.id
          ? 'text-primary bg-primary/8 before:absolute before:left-0 before:top-0 before:bottom-0 before:w-0.5 before:bg-primary before:rounded-r'
          : 'text-base-content/45 hover:text-base-content/80 hover:bg-base-300/40'"
        @click="$emit('navigate', item.id)"
      >
        <span
          class="text-xs w-4 text-center shrink-0"
          :class="current === item.id ? 'text-primary' : 'text-base-content/25'"
        >{{ item.symbol }}</span>
        <span class="font-mono text-[11px] tracking-wide">{{ item.label }}</span>
      </button>
    </nav>

    <!-- Spacer -->
    <div class="flex-1" />

    <!-- Bottom bar: connection + theme toggle -->
    <div class="px-4 py-3 border-t border-base-300/60 flex items-center justify-between">
      <div class="flex items-center gap-2">
        <span
          class="w-1.5 h-1.5 rounded-full shrink-0"
          :class="connected ? 'bg-success shadow-[0_0_4px_theme(colors.success)]' : 'bg-error'"
        />
        <span class="text-[11px] font-mono" :class="connected ? 'text-success/60' : 'text-error/60'">
          {{ connected ? 'live' : 'off' }}
        </span>
      </div>
      <button
        class="text-base-content/40 hover:text-base-content/80 transition-colors text-base leading-none"
        :title="isDark() ? 'Switch to light mode' : 'Switch to dark mode'"
        @click="toggleTheme"
      >{{ isDark() ? '🌙' : '☀️' }}</button>
    </div>

  </aside>
</template>

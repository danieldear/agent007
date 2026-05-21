<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

defineProps({ current: String, connected: Boolean })
defineEmits(['navigate'])

const { api } = useApi()
const projectName = ref('')
const projectPath = ref('')

// 'night' is our custom amber dark theme; 'day' is the warm light theme
const DARK_THEME = 'night'
const LIGHT_THEME = 'day'

const THEMES = [
  { id: 'night',     label: '🌙 Night',     dark: true  },
  { id: 'forest',    label: '🌿 Forest',    dark: true  },
  { id: 'ocean',     label: '🌊 Ocean',     dark: true  },
  { id: 'aurora',    label: '✨ Aurora',    dark: true  },
  { id: 'day',       label: '☀️ Day',       dark: false },
  { id: 'corporate', label: '💼 Corporate', dark: false },
]

function normalizeSaved(t) {
  // Migrate old 'dark' → 'night', old 'light' → 'day'
  if (!t || t === 'dark') return DARK_THEME
  if (t === 'light') return LIGHT_THEME
  return t
}

const theme = ref(normalizeSaved(localStorage.getItem('theme')) || document.documentElement.getAttribute('data-theme') || DARK_THEME)
const showThemePicker = ref(false)

function applyTheme(t) {
  document.documentElement.setAttribute('data-theme', t)
  localStorage.setItem('theme', t)
  theme.value = t
  showThemePicker.value = false
}

function isDark(id) {
  return THEMES.find(t => t.id === id)?.dark ?? true
}

function toggleDark() {
  if (isDark(theme.value)) {
    applyTheme(LIGHT_THEME)
  } else {
    applyTheme(DARK_THEME)
  }
}

onMounted(() => {
  const saved = normalizeSaved(localStorage.getItem('theme'))
  if (saved) {
    applyTheme(saved)
  } else {
    theme.value = document.documentElement.getAttribute('data-theme') || DARK_THEME
  }
})

onMounted(async () => {
  try {
    const stats = await api.getStats()
    if (stats?.project_name) projectName.value = stats.project_name
    if (stats?.project_path) projectPath.value = stats.project_path
  } catch {}
})

const primaryNav = [
  { id: 'dashboard', label: 'Dashboard', symbol: '▣' },
  { id: 'agents',    label: 'Personas',  symbol: '◉' },
  { id: 'skills',    label: 'Skills',    symbol: '⚡' },
  { id: 'workflows', label: 'Workflows', symbol: '⬡' },
  { id: 'memory',    label: 'Memory',    symbol: '◈' },
]

const configNav = [
  { id: 'tools',      label: 'Tools',       symbol: '🛠' },
  { id: 'mcp',        label: 'MCP',         symbol: '⬡' },
  { id: 'lsp',        label: 'LSP',         symbol: '⌘' },
  { id: 'extensions', label: 'Extensions',  symbol: '⊞' },
  { id: 'sharing',    label: 'Sharing',     symbol: '⇅' },
  { id: 'help',       label: 'Guide',       symbol: '?' },
]

</script>

<template>
  <aside class="w-52 bg-base-200 border-r border-base-300/80 flex flex-col shrink-0">
    <!-- Branding -->
    <div class="px-4 pt-5 pb-4 border-b border-base-300/80">
      <div class="flex items-baseline gap-2">
        <h1 class="text-base font-bold font-mono text-primary tracking-widest uppercase">agent007</h1>
        <span class="text-[10px] font-mono text-base-content/30 tracking-wider">v0.1</span>
      </div>
      <p class="text-[11px] text-base-content/40 mt-0.5 tracking-wide">AI Orchestration</p>
      <div v-if="projectName"
        class="mt-3 px-2 py-1 rounded border border-primary/20 bg-primary/5 flex items-center gap-1.5"
        :title="projectPath">
        <span class="text-[9px] font-mono text-primary/60 uppercase tracking-widest">proj</span>
        <span class="text-[11px] font-mono text-primary/80 truncate">{{ projectName }}</span>
      </div>
    </div>

    <!-- Navigation -->
    <nav class="flex-1 py-2 overflow-y-auto">
      <!-- Primary section -->
      <div class="px-3 pt-1 pb-0.5">
        <span class="text-[9px] font-mono uppercase tracking-widest text-base-content/20">Primary</span>
      </div>
      <div class="space-y-0.5 mb-2">
        <button
          v-for="item in primaryNav"
          :key="item.id"
          class="relative w-full text-left px-4 py-2.5 flex items-center gap-3 transition-colors duration-100"
          :class="current === item.id
            ? 'text-primary bg-primary/8 before:absolute before:left-0 before:top-0 before:bottom-0 before:w-0.5 before:bg-primary before:rounded-r'
            : 'text-base-content/55 hover:text-base-content/90 hover:bg-base-300/40'"
          :title="item.label"
          @click="$emit('navigate', item.id)"
        >
          <span class="text-sm w-4 text-center shrink-0" :class="current === item.id ? 'text-primary' : 'text-base-content/35'">
            {{ item.symbol }}
          </span>
          <span class="font-mono text-[12px] tracking-wide leading-tight">{{ item.label }}</span>
        </button>
      </div>

      <!-- Config section -->
      <div class="px-3 pt-2 pb-0.5 border-t border-base-300/40">
        <span class="text-[9px] font-mono uppercase tracking-widest text-base-content/20">Config</span>
      </div>
      <div class="space-y-0.5">
        <button
          v-for="item in configNav"
          :key="item.id"
          class="relative w-full text-left px-4 py-2 flex items-center gap-3 transition-colors duration-100"
          :class="current === item.id
            ? 'text-primary bg-primary/8 before:absolute before:left-0 before:top-0 before:bottom-0 before:w-0.5 before:bg-primary before:rounded-r'
            : 'text-base-content/55 hover:text-base-content/90 hover:bg-base-300/40'"
          :title="item.label"
          @click="$emit('navigate', item.id)"
        >
          <span class="text-sm w-4 text-center shrink-0" :class="current === item.id ? 'text-primary' : 'text-base-content/35'">
            {{ item.symbol }}
          </span>
          <span class="font-mono text-[12px] tracking-wide leading-tight">{{ item.label }}</span>
        </button>
      </div>
    </nav>

    <!-- Appearance -->
    <div class="px-3 pt-2 pb-1 border-t border-base-300/80">
      <!-- Toggle row: dark/light + theme picker button -->
      <div class="flex items-center justify-between mb-1">
        <span class="text-[9px] font-mono uppercase tracking-widest text-base-content/25">Theme</span>
        <button
          class="text-[10px] font-mono text-base-content/35 hover:text-base-content/70 transition-colors"
          :title="showThemePicker ? 'Close picker' : 'Pick theme'"
          @click="showThemePicker = !showThemePicker"
        >{{ showThemePicker ? '▲' : '▼' }}</button>
      </div>

      <!-- Quick dark/light toggle -->
      <button
        class="w-full flex items-center gap-2 px-2 py-1.5 rounded border border-base-300/60 bg-base-300/30 hover:bg-base-300/60 transition-colors"
        @click="toggleDark"
      >
        <span class="text-sm">{{ isDark(theme) ? '🌙' : '☀️' }}</span>
        <span class="font-mono text-[11px] text-base-content/60 flex-1 text-left">{{ THEMES.find(t => t.id === theme)?.label?.split(' ').slice(1).join(' ') || theme }}</span>
        <span class="text-[9px] font-mono text-base-content/25">{{ isDark(theme) ? 'dark' : 'light' }}</span>
      </button>

      <!-- Expanded theme picker -->
      <div v-if="showThemePicker" class="mt-1.5 grid grid-cols-2 gap-1">
        <button
          v-for="t in THEMES"
          :key="t.id"
          class="rounded px-1.5 py-1.5 text-[10px] font-mono leading-tight flex items-center gap-1 transition-all duration-150 truncate"
          :class="theme === t.id
            ? 'bg-primary/15 text-primary ring-1 ring-primary/50 font-bold'
            : 'bg-base-300/30 text-base-content/45 hover:bg-base-300/70 hover:text-base-content/80'"
          :title="t.label"
          @click="applyTheme(t.id)"
        >
          <span class="shrink-0">{{ t.label.split(' ')[0] }}</span>
          <span class="truncate">{{ t.label.split(' ').slice(1).join(' ') }}</span>
        </button>
      </div>
    </div>

    <!-- Connection status -->
    <div class="px-4 py-2.5 border-t border-base-300/40">
      <div class="flex items-center gap-2">
        <span
          class="w-1.5 h-1.5 rounded-full shrink-0"
          :class="connected ? 'bg-success shadow-[0_0_4px_theme(colors.success)]' : 'bg-error'"
        />
        <span class="text-[11px] font-mono" :class="connected ? 'text-success/70' : 'text-error/70'">
          {{ connected ? 'ws:live' : 'ws:off' }}
        </span>
      </div>
    </div>
  </aside>
</template>

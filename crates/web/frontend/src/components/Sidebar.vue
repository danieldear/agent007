<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

defineProps({ current: String, connected: Boolean })
defineEmits(['navigate'])

const { api } = useApi()
const projectName = ref('')
const projectPath = ref('')

onMounted(async () => {
  try {
    const stats = await api.getStats()
    if (stats?.project_name) projectName.value = stats.project_name
    if (stats?.project_path) projectPath.value = stats.project_path
  } catch {}
})

const navItems = [
  { id: 'dashboard', label: 'Dashboard', symbol: '▣' },
  { id: 'agents',    label: 'Personas',  symbol: '◉' },
  { id: 'skills',    label: 'Skills',    symbol: '⚡' },
  { id: 'workflows', label: 'Workflows', symbol: '⬡' },
  { id: 'tools',     label: 'Tools',     symbol: '🛠' },
  { id: 'mcp',        label: 'MCP',        symbol: '⬡' },
  { id: 'extensions', label: 'Extensions', symbol: '⊞' },
  { id: 'memory',     label: 'Memory',     symbol: '◈' },
  { id: 'sharing',   label: 'Sharing',   symbol: '⇅' },
  { id: 'help',      label: 'Guide',     symbol: '?' },
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
    <nav class="flex-1 py-3 space-y-0.5">
      <button
        v-for="item in navItems"
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
        <span class="font-mono text-[12px] tracking-wide leading-tight line-clamp-2 break-words">{{ item.label }}</span>
      </button>
    </nav>

    <!-- Connection status -->
    <div class="px-4 py-3 border-t border-base-300/80">
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

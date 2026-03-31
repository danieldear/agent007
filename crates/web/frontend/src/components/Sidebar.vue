<script setup>
defineProps({ current: String, connected: Boolean })
defineEmits(['navigate'])

const navItems = [
  { id: 'dashboard', label: 'Dashboard', icon: '⊞' },
  { id: 'agents', label: 'Agents', icon: '◉' },
  { id: 'skills', label: 'Skills', icon: '⚡' },
  { id: 'workflows', label: 'Workflows', icon: '⬡' },
  { id: 'memory', label: 'Memory', icon: '◈' },
]
</script>

<template>
  <aside class="w-56 bg-base-200 border-r border-base-300 flex flex-col shrink-0">
    <div class="p-4 border-b border-base-300">
      <h1 class="text-lg font-bold font-mono text-primary tracking-wide">Agent007</h1>
      <p class="text-xs text-base-content/50 mt-0.5">Orchestration Dashboard</p>
    </div>

    <nav class="flex-1 py-2">
      <button
        v-for="item in navItems"
        :key="item.id"
        class="btn btn-ghost btn-sm justify-start w-full rounded-none font-mono text-xs gap-3"
        :class="{ 'btn-active': current === item.id }"
        @click="$emit('navigate', item.id)"
      >
        <span class="text-base opacity-60">{{ item.icon }}</span>
        {{ item.label }}
      </button>
    </nav>

    <div class="p-3 border-t border-base-300">
      <div class="flex items-center gap-2 text-xs">
        <span class="w-2 h-2 rounded-full" :class="connected ? 'bg-success' : 'bg-error'" />
        <span class="text-base-content/60">{{ connected ? 'Connected' : 'Disconnected' }}</span>
      </div>
      <div class="text-xs text-base-content/40 mt-1">v0.1.0</div>
    </div>
  </aside>
</template>

<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border-2 border-teal-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-teal-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-teal-500/10 border-b border-teal-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-teal-400">⬡</span>
      <span class="font-bold text-sm text-teal-400">{{ data.agent }}</span>
      <span class="ml-auto text-teal-400/60 text-[10px]">orchestrator</span>
    </div>

    <div class="px-3 py-2 space-y-1">
      <div class="flex gap-2">
        <span class="text-base-content/40">id:</span>
        <span class="text-info">{{ id }}</span>
      </div>
      <div v-if="data.workers?.length" class="border-t border-base-300 pt-1 mt-1 space-y-0.5">
        <div class="text-base-content/40 text-[10px]">workers:</div>
        <div v-for="(w, i) in data.workers" :key="i" class="flex gap-2">
          <span class="text-teal-400">◉</span>
          <span class="text-base-content/60">{{ w }}</span>
        </div>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 40) }}{{ (data.prompt?.length || 0) > 40 ? '...' : '' }}
      </div>
    </div>

    <Handle type="source" :position="Position.Bottom" class="!bg-teal-400 !w-3 !h-3 !border-2 !border-base-300" />
  </div>
</template>

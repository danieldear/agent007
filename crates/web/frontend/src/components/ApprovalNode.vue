<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border-2 border-amber-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-amber-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-amber-500/10 border-b border-amber-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-amber-400">⏸</span>
      <span class="font-bold text-sm text-amber-400">{{ data.agent }}</span>
      <span class="ml-auto text-amber-400/60 text-[10px]">approval</span>
    </div>

    <div class="px-3 py-2 space-y-1">
      <div class="flex gap-2">
        <span class="text-base-content/40">id:</span>
        <span class="text-info">{{ id }}</span>
      </div>
      <div class="flex gap-2" v-if="data.output">
        <span class="text-base-content/40">out:</span>
        <span class="text-success">{{ data.output }}</span>
      </div>
      <div class="flex gap-2 border-t border-base-300 pt-1 mt-1">
        <span class="text-amber-400/60">⏸ awaits human approval</span>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 50) }}{{ (data.prompt?.length || 0) > 50 ? '...' : '' }}
      </div>
    </div>

    <Handle type="source" :position="Position.Bottom" class="!bg-amber-400 !w-3 !h-3 !border-2 !border-base-300" />
  </div>
</template>

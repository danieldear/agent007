<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border-2 border-warning/50 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-warning !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-warning/10 border-b border-warning/30 rounded-t-lg flex items-center gap-2">
      <span class="text-warning">⚡</span>
      <span class="font-bold text-sm text-warning">{{ data.tool || 'extract' }}</span>
      <span class="ml-auto badge badge-xs badge-success font-mono">0 tokens</span>
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
      <div v-if="data.cached" class="text-[10px] text-success/60 flex items-center gap-1">
        <span>◈</span><span>cached</span>
      </div>
    </div>

    <Handle type="source" :position="Position.Bottom" class="!bg-warning !w-3 !h-3 !border-2 !border-base-300" />
  </div>
</template>

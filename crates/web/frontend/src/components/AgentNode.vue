<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border border-base-300 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-primary !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-primary/10 border-b border-base-300 rounded-t-lg flex items-center gap-2">
      <span class="text-primary">◉</span>
      <span class="font-bold text-sm text-primary">{{ data.agent }}</span>
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
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 50) }}{{ (data.prompt?.length || 0) > 50 ? '...' : '' }}
      </div>
    </div>

    <Handle type="source" :position="Position.Bottom" class="!bg-accent !w-3 !h-3 !border-2 !border-base-300" />
  </div>
</template>

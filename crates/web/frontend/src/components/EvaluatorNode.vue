<script setup>
import { Handle, Position } from '@vue-flow/core'

const props = defineProps({ data: Object, id: String })
</script>

<template>
  <div class="bg-base-200 border-2 border-orange-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-orange-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-orange-500/10 border-b border-orange-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-orange-400">↺</span>
      <span class="font-bold text-sm text-orange-400">{{ data.agent }}</span>
      <span class="ml-auto text-orange-400/60 text-[10px]">evaluator</span>
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
      <div v-if="data.evaluate" class="space-y-0.5 border-t border-base-300 pt-1 mt-1">
        <div class="flex gap-2">
          <span class="text-base-content/40">pass→</span>
          <span class="text-green-400">{{ data.evaluate.on_pass }}</span>
        </div>
        <div class="flex gap-2">
          <span class="text-base-content/40">fail→</span>
          <span class="text-orange-400">{{ data.evaluate.on_fail }}</span>
        </div>
        <div class="flex gap-2">
          <span class="text-base-content/40">retries:</span>
          <span class="text-base-content/60">{{ data.evaluate.max_retries ?? 3 }}</span>
        </div>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 40) }}{{ (data.prompt?.length || 0) > 40 ? '...' : '' }}
      </div>
    </div>

    <div class="flex justify-around pb-1">
      <Handle id="pass" type="source" :position="Position.Bottom" class="!bg-green-400 !w-3 !h-3 !border-2 !border-base-300" style="left: 35%" />
      <Handle id="retry" type="source" :position="Position.Bottom" class="!bg-orange-400 !w-3 !h-3 !border-2 !border-base-300" style="left: 65%" />
    </div>
  </div>
</template>

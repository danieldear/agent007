<script setup>
import { Handle, Position } from '@vue-flow/core'
import { computed } from 'vue'

const props = defineProps({ data: Object, id: String })

const routes = computed(() => props.data.routes || [])
</script>

<template>
  <div class="bg-base-200 border-2 border-purple-500 rounded-lg shadow-lg min-w-48 font-mono text-xs">
    <Handle type="target" :position="Position.Top" class="!bg-purple-400 !w-3 !h-3 !border-2 !border-base-300" />

    <div class="px-3 py-2 bg-purple-500/10 border-b border-purple-500/30 rounded-t-lg flex items-center gap-2">
      <span class="text-purple-400">⑂</span>
      <span class="font-bold text-sm text-purple-400">{{ data.agent }}</span>
      <span class="ml-auto text-purple-400/60 text-[10px]">router</span>
    </div>

    <div class="px-3 py-2 space-y-1">
      <div class="flex gap-2">
        <span class="text-base-content/40">id:</span>
        <span class="text-info">{{ id }}</span>
      </div>
      <div v-if="routes.length" class="border-t border-base-300 pt-1 mt-1 space-y-0.5">
        <div v-for="(route, i) in routes" :key="i" class="flex gap-2">
          <span class="text-purple-400">{{ route.when || 'default' }}</span>
          <span class="text-base-content/40">→</span>
          <span class="text-base-content/60">{{ route.goto }}</span>
        </div>
      </div>
      <div class="text-base-content/50 truncate max-w-40" :title="data.prompt">
        {{ data.prompt?.slice(0, 40) }}{{ (data.prompt?.length || 0) > 40 ? '...' : '' }}
      </div>
    </div>

    <div class="flex justify-around pb-1">
      <Handle
        v-for="(route, i) in routes"
        :key="i"
        :id="route.goto"
        type="source"
        :position="Position.Bottom"
        class="!bg-purple-400 !w-3 !h-3 !border-2 !border-base-300"
        :style="{ left: `${((i + 1) / (routes.length + 1)) * 100}%` }"
      />
    </div>
  </div>
</template>

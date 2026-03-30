<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()
const scopes = ['global', 'user', 'project']
const activeScope = ref('global')
const keys = ref([])

onMounted(() => loadKeys('global'))

async function loadKeys(scope) {
  activeScope.value = scope
  const data = await api.listMemory(scope)
  keys.value = data || []
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="p-4 border-b border-base-300 bg-base-200">
      <h2 class="text-lg font-bold">Memory Store</h2>
    </div>

    <div class="flex-1 overflow-auto p-4 space-y-4">
      <div class="flex gap-2">
        <button
          v-for="s in scopes"
          :key="s"
          class="btn btn-sm"
          :class="activeScope === s ? 'btn-primary' : 'btn-ghost'"
          @click="loadKeys(s)"
        >{{ s }}</button>
      </div>

      <div class="bg-base-200 rounded-lg border border-base-300">
        <div class="px-4 py-2 border-b border-base-300">
          <span class="text-xs font-bold uppercase tracking-wider text-base-content/50">
            {{ activeScope }} — {{ keys.length }} keys
          </span>
        </div>
        <div class="divide-y divide-base-300">
          <div
            v-for="key in keys"
            :key="key"
            class="px-4 py-2 text-sm font-mono hover:bg-base-300/30"
          >
            <span class="text-info">{{ key }}</span>
          </div>
        </div>
        <div v-if="!keys.length" class="p-8 text-center text-base-content/40 text-sm">
          No keys in scope "{{ activeScope }}"
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const ok = ref('')

const enabled = ref(true)
const categoriesCsv = ref('code_completion,reasoning')
const servers = ref([{ language: 'rust', command: 'rust-analyzer' }])

function normalizeServersMap() {
  const map = {}
  for (const row of servers.value) {
    const k = (row.language || '').trim()
    const v = (row.command || '').trim()
    if (k && v) map[k] = v
  }
  return map
}

async function loadConfig() {
  loading.value = true
  error.value = ''
  ok.value = ''
  try {
    const cfg = await api.getLspConfig()
    enabled.value = cfg?.enabled ?? true
    categoriesCsv.value = (cfg?.inject_for_categories || ['code_completion', 'reasoning']).join(',')
    const rows = Object.entries(cfg?.servers || {}).map(([language, command]) => ({ language, command }))
    servers.value = rows.length ? rows : [{ language: '', command: '' }]
  } catch (e) {
    error.value = e.message || String(e)
  } finally {
    loading.value = false
  }
}

function addRow() {
  servers.value.push({ language: '', command: '' })
}
function removeRow(i) {
  servers.value.splice(i, 1)
  if (!servers.value.length) servers.value.push({ language: '', command: '' })
}

async function saveConfig() {
  saving.value = true
  error.value = ''
  ok.value = ''
  try {
    const inject_for_categories = categoriesCsv.value
      .split(',')
      .map(s => s.trim())
      .filter(Boolean)
    await api.setLspConfig({
      enabled: !!enabled.value,
      servers: normalizeServersMap(),
      inject_for_categories,
    })
    await loadConfig()
    ok.value = 'Saved LSP configuration.'
  } catch (e) {
    error.value = e.message || String(e)
  } finally {
    saving.value = false
  }
}

async function clearConfig() {
  saving.value = true
  error.value = ''
  ok.value = ''
  try {
    await api.clearLspConfig()
    await loadConfig()
    ok.value = 'Cleared saved LSP configuration.'
  } catch (e) {
    error.value = e.message || String(e)
  } finally {
    saving.value = false
  }
}

onMounted(loadConfig)
</script>

<template>
  <div class="p-6 space-y-4 overflow-auto h-full">
    <div class="flex items-center justify-between">
      <h1 class="text-lg font-bold font-mono">LSP Configuration</h1>
      <button class="btn btn-xs btn-ghost font-mono" :disabled="loading" @click="loadConfig">Refresh</button>
    </div>

    <div class="card bg-base-200 border border-base-300">
      <div class="card-body space-y-4">
        <label class="label cursor-pointer justify-start gap-3">
          <input type="checkbox" class="checkbox checkbox-sm checkbox-primary" v-model="enabled" />
          <span class="label-text font-mono">Enable LSP context injection</span>
        </label>

        <div>
          <label class="label"><span class="label-text font-mono text-xs">Inject for categories (comma-separated)</span></label>
          <input class="input input-bordered input-sm w-full font-mono" v-model="categoriesCsv" placeholder="code_completion,reasoning" />
        </div>

        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <label class="label py-0"><span class="label-text font-mono text-xs">Language → Server command</span></label>
            <div class="flex items-center gap-2">
              <span class="text-[11px] font-mono text-base-content/50">{{ servers.filter(s => (s.language||'').trim() && (s.command||'').trim()).length }} saved</span>
              <button class="btn btn-xs btn-outline font-mono" @click="addRow">+ Row</button>
            </div>
          </div>
          <div v-for="(row, i) in servers" :key="i" class="grid grid-cols-12 gap-2 items-center">
            <input class="input input-bordered input-sm col-span-3 font-mono" v-model="row.language" placeholder="rust" />
            <input class="input input-bordered input-sm col-span-8 font-mono" v-model="row.command" placeholder="rust-analyzer" />
            <button class="btn btn-xs btn-error btn-outline col-span-1" @click="removeRow(i)">×</button>
          </div>
        </div>

        <div class="pt-2">
          <button class="btn btn-primary btn-sm font-mono" :disabled="saving" @click="saveConfig">
            {{ saving ? 'Saving…' : 'Save LSP Config' }}
          </button>
          <button class="btn btn-ghost btn-sm font-mono ml-2" :disabled="saving" @click="clearConfig">
            Clear saved
          </button>
        </div>
        <p v-if="ok" class="text-success text-sm font-mono">{{ ok }}</p>
        <p v-if="error" class="text-error text-sm font-mono">{{ error }}</p>
      </div>
    </div>
  </div>
</template>

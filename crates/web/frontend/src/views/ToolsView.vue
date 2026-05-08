<script setup>
import { ref, onMounted, computed } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

// ── state ──────────────────────────────────────────────────────────────
const loading      = ref(false)
const tools        = ref([])
const activeTab    = ref('registry')
const selectedName = ref('')

// toast
const toast = ref(null)
let toastTimer = null
function showToast(msg, type = 'success') {
  clearTimeout(toastTimer)
  toast.value = { msg, type }
  toastTimer = setTimeout(() => { toast.value = null }, 3500)
}

// edit form
const form = ref({
  scope:         'project',
  name:          '',
  description:   '',
  runtime:       'shell',
  entrypoint:    'run.sh',
  timeout_sec:   60,
  safety:        'readonly',
  tags_csv:      '',
  args_json:     '[]',
  entry_content: '',
  overwrite:     true,
})

// import
const searchProvider      = ref('all')
const searchQuery         = ref('')
const searchResults       = ref([])
const importVersion       = ref('')
const importExecutable    = ref('')
const importScope         = ref('project')
const importGenerateSkill = ref(true)
const importLocalPath     = ref('')
const importBusy          = ref(false)
const lastSkillTrigger    = ref(null)

// discovered PATH tools
const discoveredTools     = ref([])
const discoverBusy        = ref(false)
const discoverDone        = ref(false)

// group discovered tools by category
const discoveredByCategory = computed(() => {
  const map = {}
  for (const t of discoveredTools.value) {
    if (!map[t.category]) map[t.category] = []
    map[t.category].push(t)
  }
  return map
})

// registered tool names for fast lookup
const registeredNames = computed(() => new Set(tools.value.map(t => t.name)))

// test
const testToolName = ref('')
const testArgsJson = ref('[]')
const testResult   = ref(null)
const testRunning  = ref(false)

// ── computed ────────────────────────────────────────────────────────────
const selectedTool = computed(() =>
  tools.value.find(t => t.name === selectedName.value) || null
)

const toolsByScope = computed(() => ({
  project: tools.value.filter(t => t.source === 'project'),
  global:  tools.value.filter(t => t.source === 'global'),
}))

// ── helpers ──────────────────────────────────────────────────────────────
function resetForm() {
  form.value = {
    scope: 'project', name: '', description: '', runtime: 'shell',
    entrypoint: 'run.sh', timeout_sec: 60, safety: 'readonly',
    tags_csv: '', args_json: '[]', entry_content: '', overwrite: false,
  }
}

function hydrateForm(tool) {
  form.value = {
    scope:         tool.source === 'global' ? 'global' : 'project',
    name:          tool.name || '',
    description:   tool.description || '',
    runtime:       tool.runtime || 'shell',
    entrypoint:    tool.entrypoint || '',
    timeout_sec:   tool.timeout_sec || 60,
    safety:        tool.safety || 'readonly',
    tags_csv:      Array.isArray(tool.tags) ? tool.tags.join(', ') : '',
    args_json:     JSON.stringify(tool.args || [], null, 2),
    entry_content: '',
    overwrite:     true,
  }
}

function selectTool(tool) {
  selectedName.value = tool.name
  hydrateForm(tool)
  testResult.value = null
}

function newTool() {
  selectedName.value = ''
  testResult.value = null
  resetForm()
}

function parseArgsJson(raw) {
  try {
    const parsed = JSON.parse(raw || '[]')
    if (!Array.isArray(parsed)) throw new Error('must be array')
    return parsed
  } catch (e) {
    throw new Error(`Invalid args JSON: ${e.message}`)
  }
}

// ── API ──────────────────────────────────────────────────────────────────
async function loadTools() {
  loading.value = true
  try {
    const list = await api.listTools()
    tools.value = Array.isArray(list) ? list : []
    if (selectedName.value) {
      const existing = tools.value.find(t => t.name === selectedName.value)
      if (existing) hydrateForm(existing)
      else { selectedName.value = ''; resetForm() }
    }
  } catch (e) {
    showToast(e.message || 'Failed to load tools', 'error')
  } finally {
    loading.value = false
  }
}

async function saveTool() {
  loading.value = true
  try {
    const args = parseArgsJson(form.value.args_json)
    const payload = {
      scope: form.value.scope,
      manifest: {
        name:        form.value.name.trim(),
        description: form.value.description.trim(),
        runtime:     form.value.runtime,
        entrypoint:  form.value.entrypoint.trim(),
        timeout_sec: Number(form.value.timeout_sec || 60),
        safety:      form.value.safety.trim() || 'readonly',
        tags:        form.value.tags_csv.split(',').map(t => t.trim()).filter(Boolean),
        args,
      },
      overwrite:     !!form.value.overwrite,
      entry_content: form.value.entry_content?.trim() || null,
    }
    const res = await api.saveTool(payload)
    const saved = res?.tool?.name || form.value.name
    showToast(`Saved "${saved}"`)
    selectedName.value = saved
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Save failed', 'error')
  } finally {
    loading.value = false
  }
}

async function deleteTool() {
  if (!form.value.name.trim()) return
  if (!confirm(`Delete tool "${form.value.name}"?\nThis removes the directory from disk.`)) return
  loading.value = true
  try {
    await api.deleteTool(form.value.name.trim(), form.value.scope)
    showToast(`Deleted "${form.value.name}"`)
    newTool()
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Delete failed', 'error')
  } finally {
    loading.value = false
  }
}

async function approveTool() {
  if (!form.value.name.trim()) return
  loading.value = true
  try {
    const res = await api.approveTool(form.value.name.trim(), form.value.scope, 'dashboard')
    showToast(`Approved "${res?.tool?.name || form.value.name}"`)
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Approve failed', 'error')
  } finally {
    loading.value = false
  }
}

async function runSearch() {
  if (!searchQuery.value.trim()) return
  importBusy.value = true
  try {
    const res = await api.searchTools(searchProvider.value, searchQuery.value, 20)
    searchResults.value = Array.isArray(res?.results) ? res.results : []
    if (!searchResults.value.length) showToast('No results found', 'info')
  } catch (e) {
    showToast(e.message || 'Search failed', 'error')
  } finally {
    importBusy.value = false
  }
}

function noteSkillTrigger(res) {
  if (res?.generated_skill_trigger) {
    lastSkillTrigger.value = res.generated_skill_trigger
  }
}

async function importFromSearch(item) {
  importBusy.value = true
  try {
    // If already installed on PATH, register it directly (no quarantine, no wrapper).
    const provider = item.installed_path ? 'path' : item.provider
    const res = await api.importTool({
      provider,
      package:           item.package,
      executable:        importExecutable.value.trim() || null,
      version:           importVersion.value.trim() || item.version || null,
      scope:             importScope.value,
      generate_skill:    importGenerateSkill.value,
      hash_pinning:      provider !== 'path',
      approval_required: provider !== 'path',
    })
    noteSkillTrigger(res)
    const name = res?.tool?.name || item.package
    if (provider === 'path') {
      showToast(`Registered "${name}" from PATH — skill ready`)
    } else {
      showToast(`Imported "${name}" — approve before use`)
    }
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Import failed', 'error')
  } finally {
    importBusy.value = false
  }
}

async function importLocalBinary() {
  if (!importLocalPath.value.trim()) { showToast('Local path is required', 'error'); return }
  importBusy.value = true
  try {
    const res = await api.importTool({
      provider:          'local',
      package:           '',
      local_path:        importLocalPath.value.trim(),
      scope:             importScope.value,
      generate_skill:    importGenerateSkill.value,
      hash_pinning:      true,
      approval_required: true,
    })
    noteSkillTrigger(res)
    showToast(`Imported "${res?.tool?.name || 'tool'}" — quarantined; approve to enable`)
    importLocalPath.value = ''
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Local import failed', 'error')
  } finally {
    importBusy.value = false
  }
}

async function discoverPathTools() {
  discoverBusy.value = true
  try {
    const res = await api.discoverTools()
    discoveredTools.value = Array.isArray(res?.tools) ? res.tools : []
    discoverDone.value = true
    if (!discoveredTools.value.length) showToast('No known tools found in PATH', 'info')
  } catch (e) {
    showToast(e.message || 'Discover failed', 'error')
  } finally {
    discoverBusy.value = false
  }
}

async function registerPathTool(discovered) {
  importBusy.value = true
  try {
    const res = await api.importTool({
      provider:        'path',
      package:         discovered.name,
      executable:      discovered.name,
      scope:           importScope.value,
      generate_skill:  importGenerateSkill.value,
      hash_pinning:    false,
      approval_required: false,
    })
    noteSkillTrigger(res)
    showToast(`Registered "${discovered.name}" — skill created`)
    await loadTools()
  } catch (e) {
    showToast(e.message || 'Register failed', 'error')
  } finally {
    importBusy.value = false
  }
}

async function runTest() {
  const name = (testToolName.value || form.value.name || '').trim()
  if (!name) { showToast('Select a tool to test', 'error'); return }
  testRunning.value = true
  testResult.value = null
  try {
    let args
    try { args = JSON.parse(testArgsJson.value || '[]') } catch { args = [] }
    const res = await api.testTool(name, args)
    testResult.value = res
    showToast(res?.ok ? 'Test passed' : 'Test failed', res?.ok ? 'success' : 'error')
  } catch (e) {
    showToast(e.message || 'Test error', 'error')
  } finally {
    testRunning.value = false
  }
}

// ── ETR state ───────────────────────────────────────────────────────────
const etrTools       = ref([])
const etrLoading     = ref(false)
const etrSelected    = ref(null)
const etrInputJson   = ref('{}')
const etrCompact     = ref(true)
const etrResult      = ref(null)
const etrRunning     = ref(false)
const etrCacheStats  = ref(null)

// Sample payloads keyed by tool name
const ETR_SAMPLES = {
  'etr.grep':         JSON.stringify({ pattern: 'TODO', path: '.', file_glob: '*.rs' }, null, 2),
  'etr.json_extract': JSON.stringify({ path: 'package.json', query: '$.name' }, null, 2),
  'etr.csv_slice':    JSON.stringify({ path: 'data.csv', rows: [0, 10] }, null, 2),
  'etr.glob':         JSON.stringify({ pattern: 'src/**/*.rs', root: '.' }, null, 2),
  'etr.file_stat':    JSON.stringify({ path: 'Cargo.toml' }, null, 2),
  'etr.math':         JSON.stringify({ expr: '2 + 2 * 21' }, null, 2),
  'etr.diff':         JSON.stringify({ a: 'hello world', b: 'hello rust' }, null, 2),
}

function selectEtrTool(tool) {
  etrSelected.value = tool
  etrResult.value = null
  etrInputJson.value = ETR_SAMPLES[tool.name] || '{}'
}

async function runEtrTool() {
  if (!etrSelected.value || etrRunning.value) return
  etrRunning.value = true
  etrResult.value = null
  try {
    let input = {}
    try { input = JSON.parse(etrInputJson.value) } catch { /* keep {} */ }
    etrResult.value = await api.etrCall(etrSelected.value.name, input, etrCompact.value)
  } catch (e) {
    etrResult.value = { status: 'error', error: e.message }
  } finally {
    etrRunning.value = false
  }
}

async function loadEtrTools() {
  etrLoading.value = true
  try {
    const data = await api.etrListTools()
    etrTools.value = (data?.tools || []).filter(t => t.name !== 'etr.list')
    if (etrTools.value.length && !etrSelected.value) selectEtrTool(etrTools.value[0])
    const stats = await api.etrCacheStats()
    etrCacheStats.value = stats
  } catch {}
  finally { etrLoading.value = false }
}

async function clearEtrCache() {
  await api.etrCacheClear()
  const stats = await api.etrCacheStats()
  etrCacheStats.value = stats
}

onMounted(loadTools)
</script>

<template>
  <div class="flex flex-col h-full">

    <!-- Toast -->
    <div v-if="toast" class="toast toast-top toast-end z-50 pointer-events-none">
      <div class="alert alert-sm shadow-lg" :class="{
        'alert-success': toast.type === 'success',
        'alert-info':    toast.type === 'info',
        'alert-error':   toast.type === 'error',
      }">
        <span class="text-sm font-mono">{{ toast.msg }}</span>
      </div>
    </div>

    <!-- Header -->
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div class="flex items-center gap-3">
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Tool Registry</span>
        <span class="badge badge-sm badge-ghost font-mono">{{ tools.length }}</span>
      </div>
      <div class="flex items-center gap-2">
        <button class="btn btn-xs btn-ghost font-mono" :disabled="loading" @click="loadTools" title="Refresh">
          <span :class="{ 'animate-spin inline-block': loading }">↻</span>
        </button>
        <button class="btn btn-sm btn-primary font-mono text-xs px-4"
          @click="() => { newTool(); activeTab = 'registry' }">+ new</button>
      </div>
    </div>

    <!-- Tab bar -->
    <div class="flex items-center gap-0 px-5 bg-base-200 border-b border-base-300 shrink-0">
      <button
        v-for="tab in ['registry', 'import', 'test', 'etr']"
        :key="tab"
        class="px-4 py-2.5 text-[11px] font-mono uppercase tracking-widest border-b-2 transition-colors"
        :class="activeTab === tab
          ? 'border-primary text-primary'
          : 'border-transparent text-base-content/40 hover:text-base-content/70'"
        @click="activeTab = tab; if (tab === 'etr') loadEtrTools()"
      >{{ tab === 'etr' ? '⚡ etr native' : tab }}</button>
    </div>

    <!-- ─────────────────────────────────────────────────────────────────
         REGISTRY TAB
    ──────────────────────────────────────────────────────────────────── -->
    <div v-if="activeTab === 'registry'" class="flex-1 overflow-hidden flex">

      <!-- Sidebar: tool list -->
      <aside class="w-64 shrink-0 border-r border-base-300 flex flex-col overflow-hidden bg-base-200">
        <div class="flex-1 overflow-auto p-3 space-y-4">

          <div v-if="!tools.length && !loading" class="py-10 text-center">
            <p class="text-[11px] font-mono text-base-content/25 uppercase tracking-widest">no tools yet</p>
            <p class="text-xs text-base-content/20 mt-1">Use Import or create one</p>
          </div>

          <!-- Project scope group -->
          <div v-if="toolsByScope.project.length">
            <div class="flex items-center gap-2 mb-2">
              <span class="w-0.5 h-3.5 rounded-full bg-amber-400/70 shrink-0"></span>
              <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">project</span>
              <span class="text-[10px] font-mono text-base-content/20">({{ toolsByScope.project.length }})</span>
            </div>
            <div class="space-y-1">
              <button
                v-for="tool in toolsByScope.project"
                :key="tool.name"
                class="w-full text-left px-3 py-2 rounded-lg border border-l-2 transition-colors"
                :class="selectedName === tool.name
                  ? 'border-primary/50 border-l-primary bg-primary/8'
                  : 'border-base-300 border-l-amber-400/25 bg-transparent hover:border-primary/25 hover:bg-base-300/60'"
                @click="selectTool(tool)"
              >
                <div class="flex items-center justify-between gap-1.5">
                  <span class="font-mono text-[12px] text-base-content/85 truncate">{{ tool.name }}</span>
                  <span
                    class="shrink-0 w-1.5 h-1.5 rounded-full"
                    :class="tool.approved ? 'bg-success' : 'bg-warning animate-pulse'"
                    :title="tool.approved ? 'approved' : 'quarantine'"
                  ></span>
                </div>
                <p class="text-[11px] text-base-content/40 truncate mt-0.5">{{ tool.description || '—' }}</p>
                <div class="flex flex-wrap gap-1 mt-1.5">
                  <span class="badge badge-xs badge-ghost">{{ tool.runtime }}</span>
                  <span v-if="tool.hash_pinning && tool.hash_match === false" class="badge badge-xs badge-error">hash ✗</span>
                  <span v-if="tool.source_kind && tool.source_kind !== 'manual'" class="badge badge-xs badge-outline opacity-60">{{ tool.source_kind }}</span>
                </div>
              </button>
            </div>
          </div>

          <!-- Global scope group -->
          <div v-if="toolsByScope.global.length">
            <div class="flex items-center gap-2 mb-2">
              <span class="w-0.5 h-3.5 rounded-full bg-blue-400/70 shrink-0"></span>
              <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">global</span>
              <span class="text-[10px] font-mono text-base-content/20">({{ toolsByScope.global.length }})</span>
            </div>
            <div class="space-y-1">
              <button
                v-for="tool in toolsByScope.global"
                :key="tool.name"
                class="w-full text-left px-3 py-2 rounded-lg border border-l-2 transition-colors"
                :class="selectedName === tool.name
                  ? 'border-primary/50 border-l-primary bg-primary/8'
                  : 'border-base-300 border-l-blue-400/25 bg-transparent hover:border-primary/25 hover:bg-base-300/60'"
                @click="selectTool(tool)"
              >
                <div class="flex items-center justify-between gap-1.5">
                  <span class="font-mono text-[12px] text-base-content/85 truncate">{{ tool.name }}</span>
                  <span
                    class="shrink-0 w-1.5 h-1.5 rounded-full"
                    :class="tool.approved ? 'bg-success' : 'bg-warning animate-pulse'"
                  ></span>
                </div>
                <p class="text-[11px] text-base-content/40 truncate mt-0.5">{{ tool.description || '—' }}</p>
                <div class="flex flex-wrap gap-1 mt-1.5">
                  <span class="badge badge-xs badge-ghost">{{ tool.runtime }}</span>
                  <span v-if="tool.has_collisions" class="badge badge-xs badge-warning opacity-70" title="Shadowed by project scope">shadowed</span>
                </div>
              </button>
            </div>
          </div>
        </div>
      </aside>

      <!-- Edit panel -->
      <div class="flex-1 overflow-auto p-5 space-y-5">

        <!-- Empty prompt -->
        <div v-if="!selectedName && !form.name" class="h-full flex flex-col items-center justify-center gap-3 text-base-content/20">
          <span class="text-5xl">🛠</span>
          <p class="text-xs font-mono uppercase tracking-widest">select a tool or create a new one</p>
        </div>

        <template v-else>
          <!-- Status bar -->
          <div v-if="selectedTool" class="flex flex-wrap items-center gap-2 px-3 py-2.5 rounded-xl bg-base-200 border border-base-300">
            <span class="badge text-xs" :class="selectedTool.approved ? 'badge-success' : 'badge-warning'">
              {{ selectedTool.approved ? 'approved' : 'quarantine' }}
            </span>
            <span class="badge badge-ghost text-[11px] font-mono">{{ selectedTool.source_kind || 'manual' }}</span>
            <span v-if="selectedTool.source_ref" class="text-[11px] font-mono text-base-content/40 truncate max-w-xs">{{ selectedTool.source_ref }}</span>
            <span v-if="selectedTool.hash_pinning" class="badge text-[11px]"
              :class="selectedTool.hash_match === false ? 'badge-error' : 'badge-ghost'">
              {{ selectedTool.hash_match === false ? 'hash mismatch' : 'hash pinned' }}
            </span>
            <div class="flex-1"></div>
            <button v-if="!selectedTool.approved" class="btn btn-xs btn-success" :disabled="loading" @click="approveTool">
              approve
            </button>
          </div>

          <!-- Section: Manifest -->
          <div>
            <div class="flex items-center gap-2 mb-3">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Manifest</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
            </div>
            <div class="grid grid-cols-2 gap-3">
              <label class="form-control col-span-2">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">name</span>
                <input class="input input-sm input-bordered font-mono mt-1" v-model="form.name" placeholder="my-tool" />
              </label>
              <label class="form-control col-span-2">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">description</span>
                <input class="input input-sm input-bordered mt-1" v-model="form.description" placeholder="What this tool does" />
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">scope</span>
                <select class="select select-sm select-bordered mt-1" v-model="form.scope">
                  <option value="project">project</option>
                  <option value="global">global</option>
                </select>
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">runtime</span>
                <select class="select select-sm select-bordered mt-1" v-model="form.runtime">
                  <option value="shell">shell</option>
                  <option value="python">python</option>
                  <option value="node">node</option>
                  <option value="binary">binary</option>
                </select>
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">entrypoint</span>
                <input class="input input-sm input-bordered font-mono mt-1" v-model="form.entrypoint" placeholder="run.sh" />
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">timeout (sec)</span>
                <input class="input input-sm input-bordered font-mono mt-1" v-model.number="form.timeout_sec" type="number" min="1" max="3600" />
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">safety</span>
                <select class="select select-sm select-bordered mt-1" v-model="form.safety">
                  <option value="readonly">readonly</option>
                  <option value="write">write</option>
                  <option value="network">network</option>
                  <option value="admin">admin</option>
                </select>
              </label>
              <label class="form-control">
                <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">tags (csv)</span>
                <input class="input input-sm input-bordered mt-1" v-model="form.tags_csv" placeholder="tag1, tag2" />
              </label>
            </div>
          </div>

          <!-- Section: Args schema -->
          <div>
            <div class="flex items-center gap-2 mb-2">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Args Schema</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
            </div>
            <textarea
              class="textarea textarea-bordered w-full h-24 font-mono text-xs"
              v-model="form.args_json"
              placeholder='[{"name":"target","type":"string","required":true,"description":"..."}]'
            />
          </div>

          <!-- Section: Entry content -->
          <div>
            <div class="flex items-center gap-2 mb-2">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Entry Content</span>
              <span class="text-[10px] text-base-content/20 tracking-normal normal-case">(leave empty to keep existing file)</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
            </div>
            <textarea
              class="textarea textarea-bordered w-full h-28 font-mono text-xs"
              v-model="form.entry_content"
              placeholder="#!/usr/bin/env sh&#10;set -e&#10;echo hello"
            />
          </div>

          <!-- Section: Associations -->
          <div v-if="selectedTool?.associations" class="px-3 py-2.5 rounded-xl bg-base-200 border border-base-300">
            <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Associations</div>
            <div class="flex flex-wrap gap-1.5">
              <span v-for="s in selectedTool.associations.skills || []" :key="s" class="badge badge-xs badge-outline">⚡ {{ s }}</span>
              <span v-for="w in selectedTool.associations.workflows || []" :key="w" class="badge badge-xs badge-outline">⬡ {{ w }}</span>
              <span
                v-if="!(selectedTool.associations.skills?.length || selectedTool.associations.workflows?.length)"
                class="text-[11px] text-base-content/25 font-mono"
              >none</span>
            </div>
          </div>

          <!-- Actions -->
          <div class="flex flex-wrap items-center gap-3 pt-1 border-t border-base-content/8">
            <label class="label cursor-pointer gap-2 p-0">
              <input type="checkbox" class="toggle toggle-xs" v-model="form.overwrite" />
              <span class="label-text text-xs text-base-content/45">overwrite existing</span>
            </label>
            <div class="flex gap-2 ml-auto">
              <button class="btn btn-sm btn-ghost font-mono text-xs" @click="newTool">clear</button>
              <button class="btn btn-sm btn-error btn-outline font-mono text-xs"
                :disabled="loading || !form.name" @click="deleteTool">delete</button>
              <button class="btn btn-sm btn-primary font-mono text-xs"
                :disabled="loading" @click="saveTool">save</button>
            </div>
          </div>
        </template>
      </div>
    </div>

    <!-- ─────────────────────────────────────────────────────────────────
         IMPORT TAB
    ──────────────────────────────────────────────────────────────────── -->
    <div v-if="activeTab === 'import'" class="flex-1 overflow-auto p-5 space-y-6 max-w-3xl">

      <!-- Post-import skill hint -->
      <div v-if="lastSkillTrigger"
        class="flex items-center gap-3 px-4 py-3 rounded-xl bg-success/10 border border-success/30"
      >
        <span class="text-success text-base shrink-0">⚡</span>
        <div class="flex-1 min-w-0">
          <p class="text-xs text-success/80 font-mono">Companion skill created: <strong>{{ lastSkillTrigger }}</strong></p>
          <p class="text-[11px] text-base-content/45 mt-0.5">Use this trigger in Claude/Codex to invoke the tool via the LLM.</p>
        </div>
        <button class="btn btn-xs btn-ghost text-success/60" @click="lastSkillTrigger = null">✕</button>
      </div>

      <!-- Import options bar -->
      <div class="p-4 rounded-xl bg-base-200 border border-base-300 space-y-3">
        <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Import Options</div>
        <div class="grid grid-cols-3 gap-3">
          <label class="form-control">
            <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">scope</span>
            <select class="select select-sm select-bordered mt-1" v-model="importScope">
              <option value="project">project</option>
              <option value="global">global</option>
            </select>
          </label>
          <label class="form-control">
            <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">version pin</span>
            <input class="input input-sm input-bordered font-mono mt-1" v-model="importVersion" placeholder="1.2.3" />
          </label>
          <label class="form-control">
            <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35">executable</span>
            <input class="input input-sm input-bordered font-mono mt-1" v-model="importExecutable" placeholder="bin-name" />
          </label>
        </div>
        <label class="label cursor-pointer gap-2 p-0 justify-start">
          <input type="checkbox" class="toggle toggle-xs toggle-primary" v-model="importGenerateSkill" />
          <span class="label-text text-xs text-base-content/55">auto-generate companion skill so LLM can invoke this tool</span>
        </label>
      </div>

      <!-- ── 1. Discover installed PATH tools ──────────────────────── -->
      <div class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Installed on this System</span>
          <div class="flex-1 h-px bg-base-content/8"></div>
          <button class="btn btn-xs btn-ghost font-mono" :disabled="discoverBusy" @click="discoverPathTools">
            <span v-if="discoverBusy" class="loading loading-spinner loading-xs"></span>
            <span v-else>{{ discoverDone ? '↻ rescan' : 'scan PATH' }}</span>
          </button>
        </div>

        <div v-if="!discoverDone" class="py-6 text-center">
          <p class="text-[11px] font-mono text-base-content/25">Click "scan PATH" to discover installed CLI tools</p>
        </div>

        <div v-else-if="!discoveredTools.length" class="py-6 text-center">
          <p class="text-[11px] font-mono text-base-content/25">No common tools found in PATH</p>
        </div>

        <template v-else>
          <div v-for="(items, category) in discoveredByCategory" :key="category" class="space-y-1">
            <div class="flex items-center gap-2 mb-1.5">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/25">{{ category }}</span>
              <div class="flex-1 h-px bg-base-content/5"></div>
            </div>
            <div class="grid grid-cols-1 sm:grid-cols-2 gap-1.5">
              <div
                v-for="tool in items"
                :key="tool.name"
                class="flex items-center gap-2.5 px-3 py-2 rounded-lg border transition-colors"
                :class="registeredNames.has(tool.name)
                  ? 'border-success/20 bg-success/5'
                  : 'border-base-300 bg-base-200 hover:border-primary/30'"
              >
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-1.5">
                    <span class="font-mono text-[12px] text-base-content/85">{{ tool.name }}</span>
                    <span v-if="tool.version" class="text-[10px] font-mono text-base-content/25 truncate">{{ tool.version.split(' ').slice(0,2).join(' ') }}</span>
                  </div>
                  <p class="text-[11px] text-base-content/40 truncate">{{ tool.description }}</p>
                </div>
                <span v-if="registeredNames.has(tool.name)" class="badge badge-xs badge-success shrink-0">registered</span>
                <button
                  v-else
                  class="btn btn-xs btn-primary shrink-0"
                  :disabled="importBusy"
                  @click="registerPathTool(tool)"
                >register</button>
              </div>
            </div>
          </div>
        </template>
      </div>

      <!-- ── 2. Package registry search ──────────────────────────── -->
      <div class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Search Package Registries</span>
          <div class="flex-1 h-px bg-base-content/8"></div>
        </div>
        <p class="text-[11px] text-base-content/30 font-mono">
          Searches crates.io · npm · GitHub · Homebrew · PyPI. Green "✓ installed" means the binary is already in your PATH — it will be registered directly without a wrapper.
        </p>
        <div class="flex items-center gap-2">
          <select class="select select-sm select-bordered w-28 font-mono" v-model="searchProvider">
            <option value="all">all</option>
            <option value="crates">crates</option>
            <option value="npm">npm</option>
            <option value="github">github</option>
            <option value="brew">brew</option>
            <option value="pypi">pypi</option>
          </select>
          <input
            class="input input-sm input-bordered flex-1 font-mono"
            v-model="searchQuery"
            placeholder="e.g. exa, ripgrep, jq …"
            @keyup.enter="runSearch"
          />
          <button class="btn btn-sm btn-primary" :disabled="importBusy" @click="runSearch">
            <span v-if="importBusy" class="loading loading-spinner loading-xs"></span>
            <span v-else>search</span>
          </button>
        </div>

        <div class="space-y-1.5 max-h-80 overflow-auto pr-1">
          <div
            v-for="item in searchResults"
            :key="`${item.provider}-${item.package}`"
            class="flex items-center gap-3 px-3 py-2.5 rounded-lg border transition-colors"
            :class="item.installed_path
              ? 'border-success/30 bg-success/5'
              : 'border-base-300 bg-base-200 hover:border-primary/25'"
          >
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="font-mono text-sm text-base-content/85 truncate">{{ item.package }}</span>
                <!-- provider badge -->
                <span class="badge badge-xs shrink-0" :class="{
                  'badge-warning': item.provider === 'crates',
                  'badge-error':   item.provider === 'npm',
                  'badge-info':    item.provider === 'github',
                  'badge-accent':  item.provider === 'brew',
                  'badge-secondary': item.provider === 'pypi',
                  'badge-ghost':   !['crates','npm','github','brew','pypi'].includes(item.provider),
                }">{{ item.provider }}</span>
                <span v-if="item.version" class="text-[11px] font-mono text-base-content/30">v{{ item.version }}</span>
                <!-- installed indicator -->
                <span v-if="item.installed_path" class="badge badge-xs badge-success">✓ installed</span>
              </div>
              <p class="text-[11px] text-base-content/40 truncate mt-0.5">{{ item.description || 'No description' }}</p>
              <p v-if="item.installed_path" class="text-[10px] font-mono text-success/50 truncate mt-0.5">{{ item.installed_path }}</p>
              <p v-else-if="item.install_cmd" class="text-[10px] font-mono text-base-content/25 truncate mt-0.5">{{ item.install_cmd }}</p>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <span v-if="item.downloads" class="text-[11px] font-mono text-base-content/25">
                {{ item.downloads >= 1000 ? Math.floor(item.downloads / 1000) + 'k' : item.downloads }}
                {{ item.provider === 'github' ? '★' : '↓' }}
              </span>
              <button
                class="btn btn-xs shrink-0"
                :class="item.installed_path ? 'btn-success' : 'btn-accent'"
                :disabled="importBusy"
                @click="importFromSearch(item)"
              >
                {{ item.installed_path ? 'register' : 'import wrapper' }}
              </button>
            </div>
          </div>
          <p v-if="!searchResults.length" class="py-6 text-center text-[11px] font-mono text-base-content/20">
            Search across crates.io · npm · GitHub · Homebrew · PyPI above
          </p>
        </div>
      </div>

      <!-- ── 3. Local binary / script ─────────────────────────────── -->
      <div class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Local Script / Binary</span>
          <div class="flex-1 h-px bg-base-content/8"></div>
        </div>
        <div class="flex items-center gap-2">
          <input
            class="input input-sm input-bordered flex-1 font-mono"
            v-model="importLocalPath"
            placeholder="/absolute/path/to/script-or-binary"
          />
          <button class="btn btn-sm btn-secondary" :disabled="importBusy" @click="importLocalBinary">
            <span v-if="importBusy" class="loading loading-spinner loading-xs"></span>
            <span v-else>import</span>
          </button>
        </div>
        <p class="text-[11px] text-base-content/30 font-mono">
          File is copied into the tool directory and placed in quarantine. Review then Approve.
          Runtime inferred from extension: <span class="text-base-content/45">.sh → shell · .py → python · .js → node · other → binary</span>
        </p>
      </div>
    </div>

    <!-- ─────────────────────────────────────────────────────────────────
         TEST TAB
    ──────────────────────────────────────────────────────────────────── -->
    <div v-if="activeTab === 'test'" class="flex-1 overflow-auto p-5 max-w-3xl space-y-5">

      <div class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Invocation</span>
          <div class="flex-1 h-px bg-base-content/8"></div>
        </div>
        <div class="flex items-center gap-2">
          <select class="select select-sm select-bordered flex-1 font-mono" v-model="testToolName">
            <option value="">— select tool —</option>
            <option v-for="t in tools" :key="t.name" :value="t.name">
              {{ t.name }} ({{ t.source }}){{ !t.approved ? ' ⚠ quarantine' : '' }}
            </option>
          </select>
          <button
            class="btn btn-sm btn-primary"
            :disabled="testRunning || !testToolName"
            @click="runTest"
          >
            <span v-if="testRunning" class="loading loading-spinner loading-xs"></span>
            <span v-else>run</span>
          </button>
        </div>
        <label class="form-control">
          <span class="label-text text-[11px] font-mono uppercase tracking-wider text-base-content/35 mb-1">
            args (JSON array or object)
          </span>
          <textarea class="textarea textarea-bordered h-20 font-mono text-xs" v-model="testArgsJson" placeholder="[]" />
        </label>
      </div>

      <!-- Result -->
      <div v-if="testResult" class="space-y-3">
        <div class="flex items-center gap-2">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Result</span>
          <span class="badge badge-sm" :class="testResult.ok ? 'badge-success' : 'badge-error'">
            {{ testResult.ok ? 'pass' : 'fail' }}
          </span>
          <span class="text-[11px] font-mono text-base-content/30">
            exit={{ testResult.exit_code ?? '—' }} · {{ testResult.duration_ms }}ms<span v-if="testResult.timed_out"> · timed out</span>
          </span>
          <div class="flex-1 h-px bg-base-content/8"></div>
        </div>
        <div class="grid grid-cols-2 gap-3">
          <div class="rounded-xl bg-base-200 border border-base-300 p-3">
            <p class="text-[10px] font-mono uppercase text-base-content/25 mb-2">stdout</p>
            <pre class="text-[11px] font-mono whitespace-pre-wrap text-base-content/75 max-h-52 overflow-auto">{{ testResult.stdout || '(empty)' }}</pre>
          </div>
          <div class="rounded-xl bg-base-200 border border-base-300 p-3">
            <p class="text-[10px] font-mono uppercase text-base-content/25 mb-2">stderr</p>
            <pre class="text-[11px] font-mono whitespace-pre-wrap text-base-content/75 max-h-52 overflow-auto">{{ testResult.stderr || '(empty)' }}</pre>
          </div>
        </div>
        <div v-if="testResult.command?.length" class="px-3 py-2.5 rounded-lg bg-base-300/50 border border-base-content/8">
          <p class="text-[10px] font-mono uppercase text-base-content/20 mb-1">command</p>
          <code class="text-[11px] font-mono text-base-content/50 break-all">{{ testResult.command.join(' ') }}</code>
        </div>
      </div>

      <div v-else-if="!testRunning" class="flex flex-col items-center justify-center py-16 gap-2 text-base-content/20">
        <p class="text-xs font-mono uppercase tracking-widest">select a tool and run it</p>
      </div>

    </div>

    <!-- ─────────────────────────────────────────────────────────────────
         ETR NATIVE TAB
    ──────────────────────────────────────────────────────────────────── -->
    <div v-if="activeTab === 'etr'" class="flex-1 overflow-hidden flex">

      <!-- Left: tool list -->
      <aside class="w-56 shrink-0 border-r border-base-300 flex flex-col overflow-hidden bg-base-200">
        <div class="px-3 py-2.5 border-b border-base-300 flex items-center justify-between">
          <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/35">Native L1 Tools</span>
          <span class="badge badge-xs badge-warning font-mono">built-in</span>
        </div>

        <!-- cache stats -->
        <div v-if="etrCacheStats" class="px-3 py-2 border-b border-base-300/60 bg-base-300/20 flex items-center gap-2">
          <span class="text-[9px] font-mono text-base-content/30 uppercase tracking-widest">cache</span>
          <span class="text-[10px] font-mono text-base-content/50">{{ etrCacheStats.entries }} entries</span>
          <span class="text-[10px] font-mono text-base-content/30">·</span>
          <span class="text-[10px] font-mono text-base-content/40">{{ (etrCacheStats.size_bytes / 1024).toFixed(1) }}KB</span>
          <button v-if="etrCacheStats.entries > 0" class="ml-auto text-[9px] font-mono text-error/50 hover:text-error transition-colors" @click="clearEtrCache" title="Clear cache">✕</button>
        </div>

        <div v-if="etrLoading" class="py-10 flex justify-center">
          <span class="loading loading-spinner loading-sm text-primary/40"></span>
        </div>
        <div v-else class="flex-1 overflow-auto py-1">
          <button
            v-for="tool in etrTools"
            :key="tool.name"
            class="w-full text-left px-3 py-2.5 flex flex-col gap-0.5 transition-colors"
            :class="etrSelected?.name === tool.name
              ? 'bg-primary/10 border-l-2 border-primary'
              : 'hover:bg-base-300/50 border-l-2 border-transparent'"
            @click="selectEtrTool(tool)"
          >
            <div class="flex items-center gap-1.5">
              <span class="text-[9px] font-mono text-warning/70">⚡</span>
              <span class="text-[11px] font-mono text-base-content/80 font-medium">{{ tool.name }}</span>
            </div>
            <p class="text-[10px] text-base-content/40 truncate pl-3.5">{{ tool.description }}</p>
          </button>
        </div>
      </aside>

      <!-- Right: test console -->
      <div class="flex-1 overflow-auto p-5 space-y-4">
        <div v-if="!etrSelected" class="flex flex-col items-center justify-center h-full gap-2 text-base-content/20">
          <span class="text-4xl">⚡</span>
          <p class="text-xs font-mono uppercase tracking-widest">select a native tool</p>
        </div>

        <template v-else>
          <!-- Tool header -->
          <div class="flex items-start justify-between">
            <div>
              <div class="flex items-center gap-2">
                <span class="text-warning/70">⚡</span>
                <h2 class="font-mono text-sm font-bold text-base-content/80">{{ etrSelected.name }}</h2>
                <span class="badge badge-xs badge-warning font-mono">L1 · Rust</span>
                <span class="badge badge-xs badge-success font-mono">0 tokens</span>
              </div>
              <p class="text-[11px] text-base-content/45 mt-1 pl-5">{{ etrSelected.description }}</p>
            </div>
          </div>

          <!-- Input -->
          <div class="space-y-1.5">
            <div class="flex items-center gap-2">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Input JSON</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
              <label class="flex items-center gap-1.5 cursor-pointer">
                <input type="checkbox" class="checkbox checkbox-xs" v-model="etrCompact" />
                <span class="text-[10px] font-mono text-base-content/40">compact output</span>
              </label>
            </div>
            <textarea
              class="textarea textarea-bordered w-full h-28 font-mono text-xs resize-none"
              v-model="etrInputJson"
              placeholder="{}"
              spellcheck="false"
            />
          </div>

          <!-- Run button -->
          <button
            class="btn btn-sm btn-warning font-mono gap-1.5"
            :disabled="etrRunning"
            @click="runEtrTool"
          >
            <span v-if="etrRunning" class="loading loading-spinner loading-xs"></span>
            <span v-else>⚡</span>
            {{ etrRunning ? 'running...' : 'run tool' }}
          </button>

          <!-- Result -->
          <div v-if="etrResult" class="space-y-3">
            <div class="flex items-center gap-2">
              <span class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Result</span>
              <span class="badge badge-sm font-mono"
                :class="etrResult.status === 'ok' ? 'badge-success' : etrResult.status === 'denied' ? 'badge-warning' : 'badge-error'">
                {{ etrResult.status || 'error' }}
              </span>
              <span v-if="etrResult.latency_ms != null" class="text-[11px] font-mono text-base-content/30">
                {{ etrResult.latency_ms }}ms
              </span>
              <span v-if="etrResult.truncated" class="text-[10px] font-mono text-warning/60">truncated</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
            </div>
            <div class="rounded-xl bg-base-200 border border-base-300 p-4">
              <pre class="text-[11px] font-mono whitespace-pre-wrap text-base-content/75 max-h-64 overflow-auto">{{ typeof etrResult.output === 'string' ? etrResult.output : JSON.stringify(etrResult.output, null, 2) }}</pre>
            </div>
            <div v-if="etrResult.error" class="px-3 py-2 rounded-lg bg-error/10 border border-error/20">
              <p class="text-[11px] font-mono text-error/70">{{ etrResult.error }}</p>
            </div>
          </div>
        </template>
      </div>

    </div>
  </div>
</template>

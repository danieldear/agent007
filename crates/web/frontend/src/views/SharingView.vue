<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

// ─── data ────────────────────────────────────────────────────────────────────
const skills = ref([])
const workflows = ref([])
const personas = ref([])

// ─── export state ────────────────────────────────────────────────────────────
const selectedSkills = ref([])
const selectedWorkflows = ref([])
const selectedPersonas = ref([])
const selectedTools = ref([])
const exportStatus = ref(null)
const exporting = ref(false)

const allSkillsSelected = computed(
  () => skills.value.length > 0 && selectedSkills.value.length === skills.value.length
)
const allWorkflowsSelected = computed(
  () => workflows.value.length > 0 && selectedWorkflows.value.length === workflows.value.length
)
const allPersonasSelected = computed(
  () => personas.value.length > 0 && selectedPersonas.value.length === personas.value.length
)

// All unique tools/scripts discovered across all skills and workflows.
// API paths: tools/ml/infer.py → bundle key: ml/infer.py
//            scripts/train.py  → bundle key: scripts/train.py
function apiPathToBundleKey(path) {
  return path.startsWith('tools/') ? path.slice('tools/'.length) : path
}

const allTools = computed(() => {
  const seen = new Map()
  for (const s of skills.value) {
    for (const t of (s.associations?.tools || [])) {
      const key = apiPathToBundleKey(t)
      if (!seen.has(key)) seen.set(key, { key, display: t, type: 'tool', source: s.source })
    }
    for (const sc of (s.associations?.scripts || [])) {
      const key = apiPathToBundleKey(sc)
      if (!seen.has(key)) seen.set(key, { key, display: sc, type: 'script', source: s.source })
    }
  }
  for (const w of workflows.value) {
    for (const t of (w.associations?.tools || [])) {
      const key = apiPathToBundleKey(t)
      if (!seen.has(key)) seen.set(key, { key, display: t, type: 'tool', source: w.source })
    }
  }
  return [...seen.values()]
})

const allToolsSelected = computed(
  () => allTools.value.length > 0 && selectedTools.value.length === allTools.value.length
)

function toggleAllSkills() {
  selectedSkills.value = allSkillsSelected.value
    ? []
    : skills.value.map(s => s.trigger.replace(/^\//, ''))
}
function toggleAllWorkflows() {
  selectedWorkflows.value = allWorkflowsSelected.value ? [] : workflows.value.map(w => w.name)
}
function toggleAllPersonas() {
  selectedPersonas.value = allPersonasSelected.value ? [] : personas.value.map(p => p.name)
}
function toggleAllTools() {
  selectedTools.value = allToolsSelected.value ? [] : allTools.value.map(t => t.key)
}

// ─── auto-select tools from selected skills/workflows ───────────────────────
watch([selectedSkills, selectedWorkflows, skills, workflows], () => {
  const auto = new Set()
  for (const s of skills.value) {
    const key = s.trigger.replace(/^\//, '')
    if (!selectedSkills.value.includes(key)) continue
    for (const t of (s.associations?.tools || [])) auto.add(apiPathToBundleKey(t))
    for (const sc of (s.associations?.scripts || [])) auto.add(apiPathToBundleKey(sc))
  }
  for (const w of workflows.value) {
    if (!selectedWorkflows.value.includes(w.name)) continue
    for (const t of (w.associations?.tools || [])) auto.add(apiPathToBundleKey(t))
  }
  selectedTools.value = [...new Set([...selectedTools.value.filter(k => auto.has(k)), ...auto])]
}, { deep: true })

// ─── cascade: selecting a workflow auto-selects its skill and persona deps ───
watch(selectedWorkflows, (newVal, oldVal) => {
  const added = newVal.filter(n => !(oldVal || []).includes(n))
  for (const name of added) {
    const wf = workflows.value.find(w => w.name === name)
    if (!wf) continue
    for (const skillRef of (wf.skill_refs || [])) {
      const normalized = skillRef.replace(/^\//, '')
      if (!selectedSkills.value.includes(normalized)) selectedSkills.value.push(normalized)
    }
    for (const agentRef of (wf.agent_refs || [])) {
      const lower = agentRef.toLowerCase()
      const persona = personas.value.find(p => p.name.toLowerCase() === lower)
      if (persona && !selectedPersonas.value.includes(persona.name)) {
        selectedPersonas.value.push(persona.name)
      }
    }
  }
}, { deep: true })

// ─── import state ────────────────────────────────────────────────────────────
const fileInput = ref(null)
const bundleData = ref(null)
const bundlePreview = ref(null)
const parseError = ref(null)
const overwrite = ref(false)
const importStatus = ref(null)
const importing = ref(false)

// ─── load data ────────────────────────────────────────────────────────────────
onMounted(async () => {
  const [sk, wf, ps] = await Promise.all([
    api.listSkills(),
    api.listWorkflows(),
    api.listPersonas(),
  ])
  if (sk) {
    skills.value = sk
    selectedSkills.value = sk.map(s => s.trigger.replace(/^\//, ''))
  }
  if (wf) {
    workflows.value = wf
    selectedWorkflows.value = wf.map(w => w.name)
  }
  if (ps) {
    personas.value = ps
    selectedPersonas.value = ps.map(p => p.name)
  }
})

function assocTools(item) {
  return item?.associations?.tools || []
}
function assocScripts(item) {
  return item?.associations?.scripts || []
}
function previewAssociations(values, limit = 2) {
  return values.slice(0, limit)
}
function compactRef(value) {
  if (!value) return ''
  const normalized = String(value).replace(/^\.?\/*/, '')
  const parts = normalized.split('/').filter(Boolean)
  if (parts.length <= 2) return normalized
  return `${parts[0]}/…/${parts[parts.length - 1]}`
}

const totalSelected = computed(
  () => selectedSkills.value.length + selectedWorkflows.value.length + selectedPersonas.value.length + selectedTools.value.length
)

// ─── export ──────────────────────────────────────────────────────────────────
async function exportBundle() {
  exporting.value = true
  exportStatus.value = null
  try {
    const res = await api.exportBundle(selectedSkills.value, selectedWorkflows.value, selectedPersonas.value, selectedTools.value)
    if (!res.ok) {
      const body = await res.json().catch(() => ({}))
      exportStatus.value = { type: 'error', message: body?.error || `Export failed (${res.status})` }
      return
    }
    const blob = await res.blob()
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'agent007-bundle.a7bundle'
    document.body.appendChild(a)
    a.click()
    document.body.removeChild(a)
    URL.revokeObjectURL(url)
    exportStatus.value = { type: 'success', message: 'Bundle downloaded!' }
    setTimeout(() => { exportStatus.value = null }, 4000)
  } catch (e) {
    exportStatus.value = { type: 'error', message: e.message || 'Export failed' }
  } finally {
    exporting.value = false
  }
}

// ─── import ──────────────────────────────────────────────────────────────────
function openFilePicker() {
  fileInput.value?.click()
}

async function handleFileChange(e) {
  bundleData.value = null
  bundlePreview.value = null
  parseError.value = null
  importStatus.value = null

  const file = e.target.files?.[0]
  if (!file) return
  e.target.value = ''

  try {
    const text = await file.text()
    const parsed = JSON.parse(text)

    const skillCount = Array.isArray(parsed.skills) ? parsed.skills.length : null
    const workflowCount = Array.isArray(parsed.workflows) ? parsed.workflows.length : null
    const toolsCount = Array.isArray(parsed.tools) ? parsed.tools.length : null
    const personasCount = Array.isArray(parsed.personas) ? parsed.personas.length : null

    if (skillCount === null && workflowCount === null && toolsCount === null && personasCount === null) {
      parseError.value = 'File does not look like an agent007 bundle (missing skills/workflows/tools/personas arrays)'
      return
    }

    bundleData.value = parsed
    bundlePreview.value = {
      skillCount: skillCount ?? 0,
      workflowCount: workflowCount ?? 0,
      toolsCount: toolsCount ?? 0,
      personasCount: personasCount ?? 0,
    }
  } catch (err) {
    parseError.value = `Could not parse bundle: ${err.message}`
  }
}

async function importBundle() {
  if (!bundleData.value) return
  importing.value = true
  importStatus.value = null
  try {
    const result = await api.importBundle(bundleData.value, overwrite.value)
    if (result) {
      const parts = []
      if (result.imported != null)    parts.push(`Imported: ${result.imported}`)
      if (result.skipped != null)     parts.push(`Skipped: ${result.skipped}`)
      if (result.overwritten != null) parts.push(`Overwritten: ${result.overwritten}`)
      importStatus.value = {
        type: 'success',
        message: parts.length ? parts.join(' · ') : 'Import complete',
      }
    } else {
      importStatus.value = { type: 'success', message: 'Import complete' }
    }
    bundleData.value = null
    bundlePreview.value = null
  } catch (e) {
    importStatus.value = { type: 'error', message: e.message || 'Import failed' }
  } finally {
    importing.value = false
  }
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div>
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Sharing</span>
        <p class="text-[11px] font-mono text-base-content/30 mt-0.5">export &amp; import .a7bundle files</p>
      </div>
    </div>

    <div class="flex-1 overflow-auto">
      <div class="p-5 lg:p-6 grid grid-cols-1 xl:grid-cols-12 gap-6 w-full">

        <!-- ── Export Card ─────────────────────────────────────────────────── -->
        <div class="card bg-base-200 border border-base-300 shadow-sm xl:col-span-8">
          <div class="card-body p-5">
            <!-- Card header -->
            <div class="flex items-center gap-2.5 mb-1">
              <div class="w-7 h-7 rounded-lg bg-primary/12 flex items-center justify-center shrink-0">
                <span class="text-primary text-sm leading-none">↓</span>
              </div>
              <div>
                <h2 class="font-bold font-mono text-sm text-base-content/80">Export Bundle</h2>
                <p class="text-[10px] font-mono text-base-content/35 uppercase tracking-wider">pack &amp; download</p>
              </div>
            </div>
            <p class="text-xs text-base-content/50 mb-4 leading-relaxed">
              Select skills, workflows, personas, and tools to bundle into a
              <code class="font-mono bg-base-300/80 px-1 rounded">.a7bundle</code> file.
              Selecting a workflow auto-selects its dependent skills and personas.
            </p>

            <!-- 4-column picker grid -->
            <div class="grid grid-cols-1 sm:grid-cols-2 2xl:grid-cols-4 gap-4">

              <!-- Skills picker -->
              <div>
                <div class="flex items-center justify-between mb-2">
                  <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">⚡ Skills</span>
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px] text-base-content/35 font-mono">all</span>
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary"
                      :checked="allSkillsSelected" @change="toggleAllSkills" />
                  </label>
                </div>
                <div v-if="!skills.length" class="text-[11px] text-base-content/30 italic py-2">No skills installed</div>
                <div class="space-y-0.5 max-h-56 overflow-y-auto pr-1">
                  <label v-for="s in skills" :key="s.trigger"
                    class="flex items-start gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1.5">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0 mt-0.5"
                      :value="s.trigger.replace(/^\//, '')" v-model="selectedSkills" />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-1.5 flex-wrap">
                        <span class="text-[11px] font-mono truncate">{{ s.trigger }}</span>
                        <span class="badge badge-xs shrink-0 font-mono"
                          :class="s.source === 'global' ? 'badge-ghost text-base-content/35' : 'badge-warning text-warning-content'">
                          {{ s.source === 'global' ? 'Global' : 'Proj' }}
                        </span>
                      </div>
                      <div v-if="s.description" class="text-[10px] text-base-content/40 leading-relaxed mt-0.5 truncate">
                        {{ s.description }}
                      </div>
                      <div v-if="assocTools(s).length || assocScripts(s).length" class="mt-1 flex items-center gap-1 flex-wrap">
                        <span v-for="tool in previewAssociations(assocTools(s), 2)" :key="`${s.trigger}-t-${tool}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-info/25 text-info/80 bg-info/5" :title="tool">
                          {{ compactRef(tool) }}</span>
                        <span v-for="script in previewAssociations(assocScripts(s), 2)" :key="`${s.trigger}-sc-${script}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-accent/25 text-accent/80 bg-accent/5" :title="script">
                          {{ compactRef(script) }}</span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <!-- Workflows picker -->
              <div>
                <div class="flex items-center justify-between mb-2">
                  <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">⬡ Workflows</span>
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px] text-base-content/35 font-mono">all</span>
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary"
                      :checked="allWorkflowsSelected" @change="toggleAllWorkflows" />
                  </label>
                </div>
                <div v-if="!workflows.length" class="text-[11px] text-base-content/30 italic py-2">No workflows saved</div>
                <div class="space-y-0.5 max-h-56 overflow-y-auto pr-1">
                  <label v-for="w in workflows" :key="w.name"
                    class="flex items-start gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1.5">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0 mt-0.5"
                      :value="w.name" v-model="selectedWorkflows" />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-1.5 flex-wrap">
                        <span class="text-[11px] font-mono truncate">{{ w.name }}</span>
                        <span class="badge badge-xs shrink-0 font-mono"
                          :class="w.source === 'global' ? 'badge-ghost text-base-content/35' : 'badge-warning text-warning-content'">
                          {{ w.source === 'global' ? 'Global' : 'Proj' }}
                        </span>
                        <span v-if="w.steps" class="text-[9px] font-mono text-base-content/35">{{ w.steps }}s</span>
                      </div>
                      <div v-if="w.description" class="text-[10px] text-base-content/40 leading-relaxed mt-0.5 truncate">
                        {{ w.description }}
                      </div>
                      <div v-if="(w.skill_refs?.length) || (w.agent_refs?.length) || assocTools(w).length"
                        class="mt-1 flex items-center gap-1 flex-wrap">
                        <span v-for="sk in previewAssociations(w.skill_refs || [], 2)" :key="`${w.name}-sk-${sk}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-secondary/25 text-secondary/80 bg-secondary/5">{{ sk }}</span>
                        <span v-for="ag in previewAssociations(w.agent_refs || [], 2)" :key="`${w.name}-ag-${ag}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-accent/25 text-accent/80 bg-accent/5" :title="`persona: ${ag}`">{{ ag }}</span>
                        <span v-for="t in previewAssociations(assocTools(w), 2)" :key="`${w.name}-t-${t}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-info/25 text-info/80 bg-info/5" :title="t">{{ compactRef(t) }}</span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <!-- Personas picker -->
              <div>
                <div class="flex items-center justify-between mb-2">
                  <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">👤 Personas</span>
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px] text-base-content/35 font-mono">all</span>
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary"
                      :checked="allPersonasSelected" @change="toggleAllPersonas" />
                  </label>
                </div>
                <div v-if="!personas.length" class="text-[11px] text-base-content/30 italic py-2">No personas installed</div>
                <div class="space-y-0.5 max-h-56 overflow-y-auto pr-1">
                  <label v-for="p in personas" :key="p.name"
                    class="flex items-start gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1.5">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0 mt-0.5"
                      :value="p.name" v-model="selectedPersonas" />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-1.5 flex-wrap">
                        <span class="text-[11px] font-mono truncate">{{ p.name }}</span>
                        <span class="badge badge-xs shrink-0 font-mono"
                          :class="p.source === 'global' ? 'badge-ghost text-base-content/35' : 'badge-warning text-warning-content'">
                          {{ p.source === 'global' ? 'Global' : 'Proj' }}
                        </span>
                      </div>
                      <div v-if="p.description" class="text-[10px] text-base-content/40 leading-relaxed mt-0.5 truncate">
                        {{ p.description }}
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <!-- Tools & Scripts picker -->
              <div>
                <div class="flex items-center justify-between mb-2">
                  <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">🛠 Tools</span>
                  <label class="flex items-center gap-1.5 cursor-pointer">
                    <span class="text-[10px] text-base-content/35 font-mono">all</span>
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary"
                      :checked="allToolsSelected" @change="toggleAllTools" />
                  </label>
                </div>
                <div v-if="!allTools.length" class="text-[11px] text-base-content/30 italic py-2">No tools detected</div>
                <div class="space-y-0.5 max-h-56 overflow-y-auto pr-1">
                  <label v-for="t in allTools" :key="t.key"
                    class="flex items-start gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1.5">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0 mt-0.5"
                      :value="t.key" v-model="selectedTools" />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-1.5 flex-wrap">
                        <span class="text-[11px] font-mono truncate" :title="t.key">{{ compactRef(t.key) }}</span>
                        <span class="badge badge-xs shrink-0 font-mono"
                          :class="t.type === 'script' ? 'badge-accent' : 'badge-info'">
                          {{ t.type }}
                        </span>
                        <span class="badge badge-xs shrink-0 font-mono"
                          :class="t.source === 'global' ? 'badge-ghost text-base-content/35' : 'badge-warning text-warning-content'">
                          {{ t.source === 'global' ? 'Global' : 'Proj' }}
                        </span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>

            </div>

            <!-- Selection summary + export button -->
            <div class="card-actions mt-5 items-center justify-between flex-wrap gap-2">
              <div class="flex gap-2 flex-wrap">
                <span v-if="selectedSkills.length" class="badge badge-primary badge-xs">
                  {{ selectedSkills.length }} skill{{ selectedSkills.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedWorkflows.length" class="badge badge-secondary badge-xs">
                  {{ selectedWorkflows.length }} workflow{{ selectedWorkflows.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedPersonas.length" class="badge badge-accent badge-xs">
                  {{ selectedPersonas.length }} persona{{ selectedPersonas.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedTools.length" class="badge badge-info badge-xs">
                  {{ selectedTools.length }} tool{{ selectedTools.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="totalSelected === 0" class="text-[11px] font-mono text-base-content/30">nothing selected</span>
              </div>
              <button class="btn btn-xs btn-primary font-mono"
                :disabled="exporting || totalSelected === 0"
                @click="exportBundle">
                <span v-if="exporting" class="loading loading-spinner loading-xs"></span>
                <span v-else class="text-xs">↓</span>
                {{ exporting ? 'Exporting…' : 'Export .a7bundle' }}
              </button>
            </div>

            <!-- Status alert -->
            <div v-if="exportStatus" class="alert mt-3 py-2 text-sm"
              :class="exportStatus.type === 'success' ? 'alert-success' : 'alert-error'">
              <svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                <path v-if="exportStatus.type === 'success'" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                <path v-else stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <span class="text-xs font-mono">{{ exportStatus.message }}</span>
            </div>
          </div>
        </div>

        <!-- ── Import Card ─────────────────────────────────────────────────── -->
        <div class="card bg-base-200 border border-base-300 shadow-sm xl:col-span-4">
          <div class="card-body p-5">
            <!-- Card header -->
            <div class="flex items-center gap-2.5 mb-1">
              <div class="w-7 h-7 rounded-lg bg-secondary/12 flex items-center justify-center shrink-0">
                <span class="text-secondary text-sm leading-none">↑</span>
              </div>
              <div>
                <h2 class="font-bold font-mono text-sm text-base-content/80">Import Bundle</h2>
                <p class="text-[10px] font-mono text-base-content/35 uppercase tracking-wider">select &amp; import</p>
              </div>
            </div>
            <p class="text-xs text-base-content/50 mb-4 leading-relaxed">
              Import a <code class="font-mono bg-base-300/80 px-1 rounded">.a7bundle</code> file to add skills,
              workflows, tools, and personas to this instance.
            </p>

            <input ref="fileInput" type="file" accept=".a7bundle,.json" class="hidden" @change="handleFileChange" />

            <!-- Drop zone -->
            <div v-if="!bundlePreview"
              class="border-2 border-dashed border-base-300 rounded-xl py-10 px-6 text-center cursor-pointer
                     transition-all duration-200 hover:border-primary/40 hover:bg-primary/4 active:scale-[0.99]"
              @click="openFilePicker">
              <div class="text-4xl mb-3 select-none opacity-50">📦</div>
              <p class="text-sm font-mono text-base-content/50">click to select a bundle</p>
              <p class="text-[11px] text-base-content/30 mt-1 font-mono">.a7bundle · .json</p>
            </div>

            <!-- Parse error -->
            <div v-if="parseError" class="alert alert-error mt-3 py-2">
              <svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <span class="text-xs font-mono">{{ parseError }}</span>
            </div>

            <!-- Bundle stats preview -->
            <div v-if="bundlePreview" class="mt-4 space-y-4">
              <div class="stats stats-horizontal bg-base-300 w-full shadow-none border border-base-content/8 rounded-xl">
                <div class="stat py-3 px-4">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Skills</div>
                  <div class="stat-value text-2xl text-primary">{{ bundlePreview.skillCount }}</div>
                </div>
                <div class="stat py-3 px-4">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Workflows</div>
                  <div class="stat-value text-2xl text-secondary">{{ bundlePreview.workflowCount }}</div>
                </div>
                <div class="stat py-3 px-4">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Tools</div>
                  <div class="stat-value text-2xl text-info">{{ bundlePreview.toolsCount }}</div>
                </div>
                <div class="stat py-3 px-4">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Personas</div>
                  <div class="stat-value text-2xl text-accent">{{ bundlePreview.personasCount }}</div>
                </div>
              </div>

              <div class="flex items-center justify-between flex-wrap gap-3">
                <label class="flex items-center gap-2 cursor-pointer">
                  <input type="checkbox" class="toggle toggle-xs toggle-warning" v-model="overwrite" />
                  <span class="text-xs font-mono text-base-content/55">overwrite existing</span>
                </label>
                <button class="btn btn-sm btn-primary font-mono" :disabled="importing" @click="importBundle">
                  <span v-if="importing" class="loading loading-spinner loading-xs"></span>
                  <span v-else class="text-xs">↑</span>
                  {{ importing ? 'Importing…' : 'Import' }}
                </button>
              </div>

              <button class="text-[10px] font-mono text-base-content/30 underline underline-offset-2 cursor-pointer bg-transparent border-0 p-0"
                @click="openFilePicker">choose a different file</button>
            </div>

            <!-- Import result -->
            <div v-if="importStatus" class="alert mt-3 py-2"
              :class="importStatus.type === 'success' ? 'alert-success' : 'alert-error'">
              <svg xmlns="http://www.w3.org/2000/svg" class="h-4 w-4 shrink-0 stroke-current" fill="none" viewBox="0 0 24 24">
                <path v-if="importStatus.type === 'success'" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
                <path v-else stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
              </svg>
              <span class="text-xs font-mono">{{ importStatus.message }}</span>
            </div>
          </div>
        </div>

        <!-- ── Format Reference ────────────────────────────────────────────── -->
        <div class="card bg-base-200 border border-base-300 xl:col-span-12">
          <div class="card-body p-5">
            <h3 class="font-bold font-mono text-[10px] uppercase tracking-widest text-base-content/40 mb-4">
              Bundle Format Reference
            </h3>
            <div class="grid grid-cols-1 md:grid-cols-5 gap-5">
              <div class="flex items-start gap-3">
                <span class="text-primary opacity-60 mt-0.5">⚡</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Skills</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    Markdown files with YAML frontmatter — trigger, name, description, model, and a prompt template using
                    <code class="bg-base-300 px-0.5 rounded">&#123;&#123;args&#125;&#125;</code>.
                  </p>
                </div>
              </div>
              <div class="flex items-start gap-3">
                <span class="text-secondary opacity-60 mt-0.5">⬡</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Workflows</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    YAML pipelines with named persona steps, optional <code class="bg-base-300 px-0.5 rounded">depends_on</code>,
                    and special node types (evaluator, router, approval, orchestrator).
                  </p>
                </div>
              </div>
              <div class="flex items-start gap-3">
                <span class="text-info opacity-60 mt-0.5">🛠</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Tools / Scripts</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    Executable assets in <code class="bg-base-300 px-0.5 rounded">tools/</code> and
                    <code class="bg-base-300 px-0.5 rounded">scripts/</code> that skills depend on —
                    auto-selected when their parent skill is selected.
                  </p>
                </div>
              </div>
              <div class="flex items-start gap-3">
                <span class="text-accent opacity-60 mt-0.5">👤</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Personas</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    TOML agent persona files referenced by workflow <code class="bg-base-300 px-0.5 rounded">agent:</code> steps.
                    Auto-selected when a workflow that uses them is selected.
                  </p>
                </div>
              </div>
              <div class="flex items-start gap-3">
                <span class="text-base-content opacity-40 mt-0.5">◈</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Container</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    JSON envelope with <code class="bg-base-300 px-0.5 rounded">version</code>,
                    <code class="bg-base-300 px-0.5 rounded">created_at</code>, and arrays of
                    <code class="bg-base-300 px-0.5 rounded">skills</code>,
                    <code class="bg-base-300 px-0.5 rounded">workflows</code>,
                    <code class="bg-base-300 px-0.5 rounded">tools</code>, and
                    <code class="bg-base-300 px-0.5 rounded">personas</code> — each with filename, content, sha256.
                  </p>
                </div>
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>
  </div>
</template>

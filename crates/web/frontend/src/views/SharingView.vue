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
  // Keep any manually added tools; drop tools whose parent was deselected
  selectedTools.value = [
    ...new Set([
      ...selectedTools.value.filter(k => auto.has(k)),
      ...auto,
    ])
  ]
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
    exportStatus.value = { type: 'success', message: 'Bundle downloaded successfully.' }
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
      parseError.value = 'Not a valid agent007 bundle — missing expected arrays.'
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
      if (result.imported != null)    parts.push(`${result.imported} imported`)
      if (result.skipped != null)     parts.push(`${result.skipped} skipped`)
      if (result.overwritten != null) parts.push(`${result.overwritten} overwritten`)
      importStatus.value = {
        type: 'success',
        message: parts.length ? parts.join(' · ') : 'Install complete.',
      }
    } else {
      importStatus.value = { type: 'success', message: 'Install complete.' }
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
  <div class="sr flex flex-col h-full">

    <!-- ── Page header ─────────────────────────────────────────────────────── -->
    <div class="sr-topbar shrink-0 px-6 py-3.5 flex items-center justify-between border-b border-base-300">
      <div class="flex items-center gap-3">
        <div class="sr-accent-bar"></div>
        <div>
          <div class="sr-page-title">Bundle Sharing</div>
          <div class="sr-page-sub">pack · ship · restore agent007 workspaces</div>
        </div>
      </div>
      <div v-if="totalSelected > 0" class="sr-staged-pill">
        {{ totalSelected }} staged
      </div>
    </div>

    <div class="flex-1 overflow-auto">
      <div class="p-5 lg:p-6 flex flex-col gap-5">

        <!-- ── Main row: export + import ─────────────────────────────────── -->
        <div class="grid grid-cols-1 xl:grid-cols-12 gap-5">

          <!-- Export panel -->
          <div class="sr-panel xl:col-span-8 flex flex-col">
            <div class="sr-panel-hd">
              <div class="flex items-center gap-2">
                <div class="sr-panel-ico" style="background:color-mix(in oklab,var(--color-info) 12%,transparent); color:var(--color-info)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
                    <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
                    <polyline points="7 10 12 15 17 10"/>
                    <line x1="12" y1="15" x2="12" y2="3"/>
                  </svg>
                </div>
                <span class="sr-panel-title">Export Bundle</span>
              </div>
              <span class="sr-panel-sub">pack &amp; download · .a7bundle</span>
            </div>

            <!-- Three-column picker grid -->
            <div class="picker-wrap flex-1">
              <!-- Skills -->
              <div class="pcol">
                <div class="pcol-hd" style="--col-accent:var(--color-info)">
                  <span class="pcol-label">Skills</span>
                  <button class="pcol-toggle" @click="toggleAllSkills">
                    {{ allSkillsSelected ? 'none' : 'all' }}
                  </button>
                </div>
                <div v-if="!skills.length" class="pcol-empty">no skills installed</div>
                <div class="plist">
                  <label v-for="s in skills" :key="s.trigger"
                    class="pitem"
                    :class="{ 'pitem-on': selectedSkills.includes(s.trigger.replace(/^\//, '')) }"
                    style="--col-accent:var(--color-info)">
                    <input type="checkbox" class="pchk"
                      :value="s.trigger.replace(/^\//, '')" v-model="selectedSkills" />
                    <div class="pitem-body">
                      <div class="pitem-row">
                        <span class="ptrigger">{{ s.trigger }}</span>
                        <span class="src-pill" :class="s.source === 'global' ? 'src-g' : 'src-p'">
                          {{ s.source === 'global' ? 'global' : 'proj' }}
                        </span>
                      </div>
                      <div v-if="s.description" class="pdesc">{{ s.description }}</div>
                      <div v-if="assocTools(s).length || assocScripts(s).length" class="passoc">
                        <span v-for="t in previewAssociations(assocTools(s), 2)" :key="t"
                          class="atag atag-tool" :title="t">{{ compactRef(t) }}</span>
                        <span v-for="sc in previewAssociations(assocScripts(s), 2)" :key="sc"
                          class="atag atag-script" :title="sc">{{ compactRef(sc) }}</span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <div class="pdivider"></div>

              <!-- Workflows -->
              <div class="pcol">
                <div class="pcol-hd" style="--col-accent:var(--color-primary)">
                  <span class="pcol-label">Workflows</span>
                  <button class="pcol-toggle" @click="toggleAllWorkflows">
                    {{ allWorkflowsSelected ? 'none' : 'all' }}
                  </button>
                </div>
                <div v-if="!workflows.length" class="pcol-empty">no workflows saved</div>
                <div class="plist">
                  <label v-for="w in workflows" :key="w.name"
                    class="pitem"
                    :class="{ 'pitem-on': selectedWorkflows.includes(w.name) }"
                    style="--col-accent:var(--color-primary)">
                    <input type="checkbox" class="pchk"
                      :value="w.name" v-model="selectedWorkflows" />
                    <div class="pitem-body">
                      <div class="pitem-row">
                        <span class="ptrigger">{{ w.name }}</span>
                        <span class="src-pill" :class="w.source === 'global' ? 'src-g' : 'src-p'">
                          {{ w.source === 'global' ? 'global' : 'proj' }}
                        </span>
                        <span v-if="w.steps" class="psteps">{{ w.steps }}s</span>
                      </div>
                      <div v-if="w.description" class="pdesc">{{ w.description }}</div>
                      <div v-if="(w.skill_refs?.length) || (w.agent_refs?.length) || assocTools(w).length" class="passoc">
                        <span v-for="sk in previewAssociations(w.skill_refs || [], 2)" :key="sk"
                          class="atag atag-skill">{{ sk }}</span>
                        <span v-for="ag in previewAssociations(w.agent_refs || [], 2)" :key="ag"
                          class="atag atag-agent" :title="`persona: ${ag}`">{{ ag }}</span>
                        <span v-for="t in previewAssociations(assocTools(w), 2)" :key="t"
                          class="atag atag-tool" :title="t">{{ compactRef(t) }}</span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>

              <div class="pdivider"></div>

              <!-- Personas -->
              <div class="pcol">
                <div class="pcol-hd" style="--col-accent:var(--color-secondary)">
                  <span class="pcol-label">Personas</span>
                  <button class="pcol-toggle" @click="toggleAllPersonas">
                    {{ allPersonasSelected ? 'none' : 'all' }}
                  </button>
                </div>
                <div v-if="!personas.length" class="pcol-empty">no personas installed</div>
                <div class="plist">
                  <label v-for="p in personas" :key="p.name"
                    class="pitem"
                    :class="{ 'pitem-on': selectedPersonas.includes(p.name) }"
                    style="--col-accent:var(--color-secondary)">
                    <input type="checkbox" class="pchk"
                      :value="p.name" v-model="selectedPersonas" />
                    <div class="pitem-body">
                      <div class="pitem-row">
                        <span class="ptrigger">{{ p.name }}</span>
                        <span class="src-pill" :class="p.source === 'global' ? 'src-g' : 'src-p'">
                          {{ p.source === 'global' ? 'global' : 'proj' }}
                        </span>
                      </div>
                      <div v-if="p.description" class="pdesc">{{ p.description }}</div>
                    </div>
                  </label>
                </div>
              </div>

              <div class="pdivider"></div>

              <!-- Tools & Scripts -->
              <div class="pcol">
                <div class="pcol-hd" style="--col-accent:var(--color-warning)">
                  <span class="pcol-label">Tools</span>
                  <button class="pcol-toggle" @click="toggleAllTools">
                    {{ allToolsSelected ? 'none' : 'all' }}
                  </button>
                </div>
                <div v-if="!allTools.length" class="pcol-empty">no tools detected</div>
                <div class="plist">
                  <label v-for="t in allTools" :key="t.key"
                    class="pitem"
                    :class="{ 'pitem-on': selectedTools.includes(t.key) }"
                    style="--col-accent:var(--color-warning)">
                    <input type="checkbox" class="pchk"
                      :value="t.key" v-model="selectedTools" />
                    <div class="pitem-body">
                      <div class="pitem-row">
                        <span class="ptrigger" :title="t.key">{{ compactRef(t.key) }}</span>
                        <span class="atag" :class="t.type === 'script' ? 'atag-script' : 'atag-tool'">{{ t.type }}</span>
                        <span class="src-pill" :class="t.source === 'global' ? 'src-g' : 'src-p'">
                          {{ t.source === 'global' ? 'global' : 'proj' }}
                        </span>
                      </div>
                    </div>
                  </label>
                </div>
              </div>
            </div>

            <!-- Action bar -->
            <div class="action-bar">
              <div class="manifest-tags">
                <span v-if="selectedSkills.length" class="mtag" style="--ac:var(--color-info)">
                  {{ selectedSkills.length }} skill{{ selectedSkills.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedWorkflows.length" class="mtag" style="--ac:var(--color-primary)">
                  {{ selectedWorkflows.length }} workflow{{ selectedWorkflows.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedPersonas.length" class="mtag" style="--ac:var(--color-secondary)">
                  {{ selectedPersonas.length }} persona{{ selectedPersonas.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="selectedTools.length" class="mtag" style="--ac:var(--color-warning)">
                  {{ selectedTools.length }} tool{{ selectedTools.length !== 1 ? 's' : '' }}
                </span>
                <span v-if="totalSelected === 0" class="manifest-empty">nothing selected</span>
              </div>
              <button class="export-btn"
                :disabled="exporting || totalSelected === 0"
                @click="exportBundle">
                <span v-if="exporting" class="loading loading-spinner loading-xs"></span>
                <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                  <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
                  <polyline points="7 10 12 15 17 10"/>
                  <line x1="12" y1="15" x2="12" y2="3"/>
                </svg>
                {{ exporting ? 'Packing…' : 'Export .a7bundle' }}
              </button>
            </div>

            <!-- Export status -->
            <div v-if="exportStatus" class="status-strip"
              :class="exportStatus.type === 'success' ? 'status-ok' : 'status-err'">
              <span>{{ exportStatus.type === 'success' ? '✓' : '✗' }}</span>
              <span>{{ exportStatus.message }}</span>
            </div>
          </div>

          <!-- Import panel -->
          <div class="sr-panel xl:col-span-4 flex flex-col">
            <div class="sr-panel-hd">
              <div class="flex items-center gap-2">
                <div class="sr-panel-ico" style="background:color-mix(in oklab,var(--color-primary) 12%,transparent); color:var(--color-primary)">
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2">
                    <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
                    <polyline points="17 8 12 3 7 8"/>
                    <line x1="12" y1="3" x2="12" y2="15"/>
                  </svg>
                </div>
                <span class="sr-panel-title">Import Bundle</span>
              </div>
              <span class="sr-panel-sub">restore · install · deploy</span>
            </div>

            <div class="import-body flex-1 flex flex-col gap-4">
              <input ref="fileInput" type="file" accept=".a7bundle,.json" class="hidden" @change="handleFileChange" />

              <!-- Drop zone -->
              <div v-if="!bundlePreview" class="drop-zone" @click="openFilePicker">
                <svg class="drop-svg" viewBox="0 0 80 80" fill="none">
                  <rect x="6" y="6" width="68" height="68" rx="5"
                    stroke="currentColor" stroke-width="1.5" stroke-dasharray="5 4"/>
                  <path d="M40 24v20M31 35l9 10 9-10" stroke="currentColor"
                    stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  <rect x="22" y="52" width="36" height="10" rx="3"
                    stroke="currentColor" stroke-width="1.5"/>
                  <line x1="28" y1="57" x2="52" y2="57"
                    stroke="currentColor" stroke-width="1" stroke-dasharray="3 2" opacity=".5"/>
                </svg>
                <p class="drop-label">drop bundle or click to browse</p>
                <p class="drop-ext">.a7bundle · .json</p>
              </div>

              <!-- Parse error -->
              <div v-if="parseError" class="status-strip status-err">
                <span>✗</span><span>{{ parseError }}</span>
              </div>

              <!-- Bundle preview -->
              <template v-if="bundlePreview">
                <div class="bstat-grid">
                  <div class="bstat">
                    <span class="bstat-val" style="color:var(--color-info)">{{ bundlePreview.skillCount }}</span>
                    <span class="bstat-lbl">skills</span>
                  </div>
                  <div class="bstat">
                    <span class="bstat-val" style="color:var(--color-primary)">{{ bundlePreview.workflowCount }}</span>
                    <span class="bstat-lbl">workflows</span>
                  </div>
                  <div class="bstat">
                    <span class="bstat-val" style="color:var(--color-warning)">{{ bundlePreview.toolsCount }}</span>
                    <span class="bstat-lbl">tools</span>
                  </div>
                  <div class="bstat">
                    <span class="bstat-val" style="color:var(--color-secondary)">{{ bundlePreview.personasCount }}</span>
                    <span class="bstat-lbl">personas</span>
                  </div>
                </div>

                <div class="import-ctrl">
                  <label class="ow-toggle">
                    <input type="checkbox" v-model="overwrite" class="sr-only" />
                    <span class="ow-track" :class="{ 'ow-on': overwrite }">
                      <span class="ow-thumb"></span>
                    </span>
                    <span class="ow-lbl">overwrite existing</span>
                  </label>
                  <button class="import-btn" :disabled="importing" @click="importBundle">
                    <span v-if="importing" class="loading loading-spinner loading-xs"></span>
                    <svg v-else width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
                      <polyline points="17 8 12 3 7 8"/>
                      <line x1="12" y1="3" x2="12" y2="15"/>
                    </svg>
                    {{ importing ? 'Installing…' : 'Install Bundle' }}
                  </button>
                </div>

                <button class="change-file" @click="openFilePicker">choose a different file</button>
              </template>

              <!-- Import result -->
              <div v-if="importStatus" class="status-strip"
                :class="importStatus.type === 'success' ? 'status-ok' : 'status-err'">
                <span>{{ importStatus.type === 'success' ? '✓' : '✗' }}</span>
                <span>{{ importStatus.message }}</span>
              </div>
            </div>
          </div>
        </div>

        <!-- ── Format Reference ──────────────────────────────────────────── -->
        <div class="sr-panel">
          <div class="ref-hd">Bundle Format Reference</div>
          <div class="ref-grid">
            <div class="ref-card">
              <div class="ref-stripe" style="background:var(--color-info)"></div>
              <div>
                <div class="ref-title" style="color:var(--color-info)">Skills</div>
                <p class="ref-body">Markdown files with YAML frontmatter — trigger, name, description, model, and a prompt template using <code>&#123;&#123;args&#125;&#125;</code>.</p>
              </div>
            </div>
            <div class="ref-card">
              <div class="ref-stripe" style="background:var(--color-primary)"></div>
              <div>
                <div class="ref-title" style="color:var(--color-primary)">Workflows</div>
                <p class="ref-body">YAML pipelines with named persona steps, optional <code>depends_on</code>, and node types: evaluator, router, approval, orchestrator.</p>
              </div>
            </div>
            <div class="ref-card">
              <div class="ref-stripe" style="background:var(--color-warning)"></div>
              <div>
                <div class="ref-title" style="color:var(--color-warning)">Tools / Scripts</div>
                <p class="ref-body">Executable assets in <code>tools/</code> and <code>scripts/</code> that skills depend on — packed and restored automatically.</p>
              </div>
            </div>
            <div class="ref-card">
              <div class="ref-stripe" style="background:var(--color-secondary)"></div>
              <div>
                <div class="ref-title" style="color:var(--color-secondary)">Personas</div>
                <p class="ref-body">TOML agent persona files referenced by workflow <code>agent:</code> fields. Collected from project-local and global homes.</p>
              </div>
            </div>
            <div class="ref-card" style="border-right:none">
              <div class="ref-stripe" style="background:var(--color-neutral-content)"></div>
              <div>
                <div class="ref-title" style="color:var(--color-neutral-content)">Container</div>
                <p class="ref-body">JSON with <code>version</code>, <code>created_at</code>, and arrays of <code>skills</code>, <code>workflows</code>, <code>tools</code>, <code>personas</code> — each with filename, content, sha256.</p>
              </div>
            </div>
          </div>
        </div>

      </div>
    </div>
  </div>
</template>

<style scoped>
@import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=Barlow+Condensed:wght@500;600;700&display=swap');

/* ── Root ─────────────────────────────────────────────────────────────────── */
.sr {
  font-family: 'IBM Plex Mono', 'JetBrains Mono', monospace;
  background-image: radial-gradient(circle, color-mix(in oklab,var(--color-base-content) 8%,transparent) 1px, transparent 1px);
  background-size: 22px 22px;
}

/* ── Page header ─────────────────────────────────────────────────────────── */
.sr-topbar {
  background: oklch(from var(--color-base-200, #1e2228) l c h);
}

.sr-accent-bar {
  width: 3px;
  height: 26px;
  border-radius: 2px;
  background: linear-gradient(180deg, var(--color-info) 0%, var(--color-primary) 50%, var(--color-secondary) 100%);
  flex-shrink: 0;
}

.sr-page-title {
  font-family: 'Barlow Condensed', sans-serif;
  font-weight: 700;
  font-size: 17px;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  opacity: .85;
}

.sr-page-sub {
  font-size: 9.5px;
  letter-spacing: 0.09em;
  opacity: .3;
  margin-top: 1px;
}

.sr-staged-pill {
  font-size: 9px;
  letter-spacing: 0.07em;
  padding: 3px 10px;
  border-radius: 2px;
  border: 1px solid color-mix(in oklab,var(--color-base-content) 20%,transparent);
  opacity: .55;
}

/* ── Panels ──────────────────────────────────────────────────────────────── */
.sr-panel {
  border-radius: 4px;
  border: 1px solid color-mix(in oklab,var(--color-base-content) 12%,transparent);
  overflow: hidden;
}

.sr-panel-hd {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid color-mix(in oklab,var(--color-base-content) 9%,transparent);
  background: rgba(0,0,0,.08);
}

.sr-panel-ico {
  width: 26px;
  height: 26px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.sr-panel-title {
  font-family: 'Barlow Condensed', sans-serif;
  font-weight: 600;
  font-size: 13px;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  opacity: .8;
}

.sr-panel-sub {
  font-size: 9px;
  letter-spacing: 0.07em;
  opacity: .28;
}

/* ── Picker grid ─────────────────────────────────────────────────────────── */
.picker-wrap {
  display: grid;
  grid-template-columns: 1fr 1px 1fr 1px 1fr 1px 1fr;
}

@media (max-width: 1100px) {
  .picker-wrap { grid-template-columns: 1fr 1px 1fr; }
}

@media (max-width: 700px) {
  .picker-wrap { grid-template-columns: 1fr; }
  .pdivider { display: none; }
}

.pdivider {
  background: color-mix(in oklab,var(--color-base-content) 8%,transparent);
}

.pcol {
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.pcol-hd {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 13px 8px;
  border-bottom: 1.5px solid var(--col-accent);
}

.pcol-label {
  font-family: 'Barlow Condensed', sans-serif;
  font-weight: 600;
  font-size: 11px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  opacity: .5;
}

.pcol-toggle {
  font-size: 9px;
  letter-spacing: 0.05em;
  opacity: .3;
  background: none;
  border: none;
  cursor: pointer;
  padding: 0;
  font-family: inherit;
  transition: opacity 0.15s;
}
.pcol-toggle:hover { opacity: .7; }

.pcol-empty {
  padding: 20px 13px;
  font-size: 10px;
  opacity: .22;
  font-style: italic;
}

.plist {
  flex: 1;
  max-height: 256px;
  overflow-y: auto;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in oklab,var(--color-base-content) 12%,transparent) transparent;
}

/* ── Picker items ─────────────────────────────────────────────────────────── */
.pitem {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 6px 13px;
  cursor: pointer;
  border-left: 2px solid transparent;
  transition: background 0.12s, border-color 0.15s;
  user-select: none;
}
.pitem:hover { background: color-mix(in oklab,var(--color-base-content) 4%,transparent); }
.pitem-on {
  border-left-color: var(--col-accent);
  background: color-mix(in srgb, var(--col-accent) 5%, transparent);
}

.pchk {
  width: 11px;
  height: 11px;
  margin-top: 3px;
  flex-shrink: 0;
  cursor: pointer;
  accent-color: var(--color-info);
}

.pitem-body { min-width: 0; flex: 1; }

.pitem-row {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
}

.ptrigger {
  font-size: 11px;
  font-weight: 500;
  opacity: .78;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 130px;
}

.src-pill {
  font-size: 8px;
  padding: 1px 5px;
  border-radius: 2px;
  letter-spacing: 0.05em;
  flex-shrink: 0;
}
.src-g { background: color-mix(in oklab,var(--color-base-content) 8%,transparent); opacity: .45; }
.src-p { background: color-mix(in oklab,var(--color-warning) 12%,transparent); color: var(--color-warning); opacity: .9; }

.psteps {
  font-size: 8px;
  opacity: .22;
  flex-shrink: 0;
}

.pdesc {
  font-size: 9px;
  opacity: .33;
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.passoc {
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
  margin-top: 3px;
}

.atag {
  font-size: 8px;
  padding: 1px 5px;
  border-radius: 2px;
  border: 1px solid;
}
.atag-tool   { border-color: color-mix(in oklab,var(--color-info) 30%,transparent);      color: var(--color-info);      background: color-mix(in oklab,var(--color-info) 8%,transparent); }
.atag-script { border-color: color-mix(in oklab,var(--color-warning) 30%,transparent);   color: var(--color-warning);   background: color-mix(in oklab,var(--color-warning) 8%,transparent); }
.atag-skill  { border-color: color-mix(in oklab,var(--color-primary) 30%,transparent);   color: var(--color-primary);   background: color-mix(in oklab,var(--color-primary) 8%,transparent); }
.atag-agent  { border-color: color-mix(in oklab,var(--color-secondary) 30%,transparent); color: var(--color-secondary); background: color-mix(in oklab,var(--color-secondary) 8%,transparent); }

/* ── Action bar ──────────────────────────────────────────────────────────── */
.action-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
  padding: 10px 14px;
  border-top: 1px solid color-mix(in oklab,var(--color-base-content) 8%,transparent);
  background: rgba(0,0,0,.06);
}

.manifest-tags {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.mtag {
  font-size: 9.5px;
  padding: 2px 8px;
  border-radius: 2px;
  border: 1px solid color-mix(in srgb, var(--ac) 30%, transparent);
  color: color-mix(in srgb, var(--ac) 85%, white);
  background: color-mix(in srgb, var(--ac) 8%, transparent);
}

.manifest-empty {
  font-size: 10px;
  opacity: .22;
  letter-spacing: 0.04em;
}

.export-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 500;
  padding: 6px 16px;
  border-radius: 3px;
  background: var(--color-info);
  color: var(--color-info-content);
  border: none;
  cursor: pointer;
  letter-spacing: 0.04em;
  transition: opacity 0.15s, transform 0.1s;
}
.export-btn:hover:not(:disabled) { opacity: .9; transform: translateY(-1px); }
.export-btn:active:not(:disabled) { transform: translateY(0); }
.export-btn:disabled { opacity: .35; cursor: not-allowed; transform: none; }

/* ── Status strips ───────────────────────────────────────────────────────── */
.status-strip {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 14px;
  font-size: 10.5px;
  border-top: 1px solid transparent;
}
.status-ok  { background: color-mix(in oklab,var(--color-success) 8%,transparent);  color: var(--color-success);  border-top-color: color-mix(in oklab,var(--color-success) 18%,transparent); }
.status-err { background: color-mix(in oklab,var(--color-error) 8%,transparent);    color: var(--color-error);    border-top-color: color-mix(in oklab,var(--color-error) 18%,transparent); }

/* ── Import body ─────────────────────────────────────────────────────────── */
.import-body {
  padding: 16px;
}

.drop-zone {
  border: 1px dashed color-mix(in oklab,var(--color-base-content) 22%,transparent);
  border-radius: 4px;
  padding: 32px 16px;
  text-align: center;
  cursor: pointer;
  transition: border-color 0.2s, background 0.2s;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}
.drop-zone:hover {
  border-color: color-mix(in oklab,var(--color-primary) 45%,transparent);
  background: color-mix(in oklab,var(--color-primary) 4%,transparent);
}

.drop-svg {
  width: 56px;
  height: 56px;
  color: color-mix(in oklab,var(--color-base-content) 22%,transparent);
  transition: color 0.2s;
}
.drop-zone:hover .drop-svg { color: color-mix(in oklab,var(--color-primary) 40%,transparent); }

.drop-label {
  font-size: 10.5px;
  opacity: .42;
  letter-spacing: 0.04em;
}
.drop-ext {
  font-size: 9px;
  opacity: .22;
  letter-spacing: 0.07em;
}

/* ── Bundle stats ─────────────────────────────────────────────────────────── */
.bstat-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}

.bstat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 12px;
  border-radius: 3px;
  border: 1px solid color-mix(in oklab,var(--color-base-content) 10%,transparent);
  background: rgba(0,0,0,.08);
}

.bstat-val {
  font-family: 'Barlow Condensed', sans-serif;
  font-size: 30px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: -0.01em;
}

.bstat-lbl {
  font-size: 8.5px;
  opacity: .32;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

/* ── Import controls ─────────────────────────────────────────────────────── */
.import-ctrl {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.ow-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}
.ow-track {
  width: 30px;
  height: 17px;
  background: color-mix(in oklab,var(--color-base-content) 15%,transparent);
  border-radius: 9px;
  position: relative;
  transition: background 0.2s;
  flex-shrink: 0;
}
.ow-on { background: color-mix(in oklab,var(--color-warning) 50%,transparent); }
.ow-thumb {
  position: absolute;
  top: 2.5px;
  left: 2.5px;
  width: 12px;
  height: 12px;
  background: white;
  border-radius: 50%;
  transition: transform 0.2s;
  box-shadow: 0 1px 3px rgba(0,0,0,.25);
}
.ow-on .ow-thumb { transform: translateX(13px); }
.ow-lbl { font-size: 9.5px; opacity: .45; }

.import-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: inherit;
  font-size: 11px;
  font-weight: 500;
  padding: 6px 14px;
  border-radius: 3px;
  background: color-mix(in oklab,var(--color-primary) 12%,transparent);
  color: var(--color-primary);
  border: 1px solid color-mix(in oklab,var(--color-primary) 28%,transparent);
  cursor: pointer;
  transition: all 0.15s;
}
.import-btn:hover:not(:disabled) {
  background: color-mix(in oklab,var(--color-primary) 22%,transparent);
  border-color: color-mix(in oklab,var(--color-primary) 50%,transparent);
}
.import-btn:disabled { opacity: .35; cursor: not-allowed; }

.change-file {
  background: none;
  border: none;
  cursor: pointer;
  font-family: inherit;
  font-size: 9px;
  opacity: .25;
  text-decoration: underline;
  text-underline-offset: 2px;
  padding: 0;
  transition: opacity 0.15s;
}
.change-file:hover { opacity: .55; }

/* ── Format reference ─────────────────────────────────────────────────────── */
.ref-hd {
  padding: 9px 14px;
  border-bottom: 1px solid color-mix(in oklab,var(--color-base-content) 8%,transparent);
  font-family: 'Barlow Condensed', sans-serif;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.13em;
  text-transform: uppercase;
  opacity: .3;
}

.ref-grid {
  display: grid;
  grid-template-columns: repeat(5, 1fr);
}

@media (max-width: 900px) {
  .ref-grid { grid-template-columns: repeat(2, 1fr); }
}

.ref-card {
  display: flex;
  gap: 10px;
  padding: 14px 14px;
  border-right: 1px solid color-mix(in oklab,var(--color-base-content) 7%,transparent);
}

.ref-stripe {
  width: 3px;
  border-radius: 2px;
  flex-shrink: 0;
  align-self: stretch;
  min-height: 50px;
  opacity: .7;
}

.ref-title {
  font-size: 11px;
  font-weight: 600;
  margin-bottom: 5px;
  letter-spacing: 0.03em;
}

.ref-body {
  font-size: 9.5px;
  line-height: 1.55;
  opacity: .42;
}

.ref-body code {
  font-family: inherit;
  font-size: 9px;
  padding: 1px 4px;
  border-radius: 2px;
  background: color-mix(in oklab,var(--color-base-content) 10%,transparent);
  opacity: .9;
}
</style>

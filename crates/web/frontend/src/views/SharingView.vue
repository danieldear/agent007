<script setup>
import { ref, computed, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

// ─── data ────────────────────────────────────────────────────────────────────
const skills = ref([])
const workflows = ref([])

// ─── export state ────────────────────────────────────────────────────────────
const selectedSkills = ref([])
const selectedWorkflows = ref([])
const exportStatus = ref(null)
const exporting = ref(false)

const allSkillsSelected = computed(
  () => skills.value.length > 0 && selectedSkills.value.length === skills.value.length
)
const allWorkflowsSelected = computed(
  () => workflows.value.length > 0 && selectedWorkflows.value.length === workflows.value.length
)

function toggleAllSkills() {
  selectedSkills.value = allSkillsSelected.value
    ? []
    : skills.value.map(s => s.trigger.replace(/^\//, ''))
}
function toggleAllWorkflows() {
  selectedWorkflows.value = allWorkflowsSelected.value ? [] : workflows.value.map(w => w.name)
}

// ─── import state ────────────────────────────────────────────────────────────
const fileInput = ref(null)
const bundleData = ref(null)
const bundlePreview = ref(null)   // { skillCount, workflowCount }
const parseError = ref(null)
const overwrite = ref(false)
const importStatus = ref(null)
const importing = ref(false)

// ─── load data ────────────────────────────────────────────────────────────────
onMounted(async () => {
  const [sk, wf] = await Promise.all([api.listSkills(), api.listWorkflows()])
  if (sk) {
    skills.value = sk
    selectedSkills.value = sk.map(s => s.trigger.replace(/^\//, ''))
  }
  if (wf) {
    workflows.value = wf
    selectedWorkflows.value = wf.map(w => w.name)
  }
})

// ─── export ──────────────────────────────────────────────────────────────────
async function exportBundle() {
  exporting.value = true
  exportStatus.value = null
  try {
    const res = await api.exportBundle(selectedSkills.value, selectedWorkflows.value)
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
  // Reset state immediately so stale info is never visible
  bundleData.value = null
  bundlePreview.value = null
  parseError.value = null
  importStatus.value = null

  const file = e.target.files?.[0]
  if (!file) return

  // Reset input so the same file can be re-selected
  e.target.value = ''

  try {
    const text = await file.text()
    const parsed = JSON.parse(text)

    // Strict shape check — bundle schema: { skills: [], workflows: [] }
    const skillCount = Array.isArray(parsed.skills) ? parsed.skills.length : null
    const workflowCount = Array.isArray(parsed.workflows) ? parsed.workflows.length : null

    if (skillCount === null && workflowCount === null) {
      parseError.value = 'File does not look like an agent007 bundle (missing skills/workflows arrays)'
      return
    }

    bundleData.value = parsed
    bundlePreview.value = {
      skillCount: skillCount ?? 0,
      workflowCount: workflowCount ?? 0,
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
    // Expected shape: { imported, skipped, overwritten } — display whatever we get
    if (result) {
      const parts = []
      if (result.imported != null)   parts.push(`Imported: ${result.imported}`)
      if (result.skipped != null)    parts.push(`Skipped: ${result.skipped}`)
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
      <div class="p-6 grid grid-cols-1 lg:grid-cols-2 gap-6 max-w-5xl">

        <!-- ── Export Card ─────────────────────────────────────────────────── -->
        <div class="card bg-base-200 border border-base-300 shadow-sm">
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
              Bundle selected skills and workflows into a <code class="font-mono bg-base-300/80 px-1 rounded">.a7bundle</code>
              file for sharing or backup on any agent007 instance.
            </p>

            <div class="grid grid-cols-2 gap-4">
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
                <div class="space-y-px max-h-44 overflow-y-auto pr-1">
                  <label v-for="s in skills" :key="s.trigger"
                    class="flex items-center gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0"
                      :value="s.trigger.replace(/^\//, '')" v-model="selectedSkills" />
                    <span class="text-[11px] font-mono truncate flex-1">{{ s.trigger }}</span>
                    <span class="badge badge-xs shrink-0 font-mono"
                      :class="s.source === 'global'
                        ? 'badge-ghost text-base-content/35'
                        : 'badge-warning text-warning-content'">
                      {{ s.source === 'global' ? 'Global' : 'Proj' }}
                    </span>
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
                <div class="space-y-px max-h-44 overflow-y-auto pr-1">
                  <label v-for="w in workflows" :key="w.name"
                    class="flex items-center gap-2 cursor-pointer hover:bg-base-300/60 rounded px-1.5 py-1">
                    <input type="checkbox" class="checkbox checkbox-xs checkbox-primary shrink-0"
                      :value="w.name" v-model="selectedWorkflows" />
                    <span class="text-[11px] font-mono truncate flex-1">{{ w.name }}</span>
                    <span class="badge badge-xs shrink-0 font-mono"
                      :class="w.source === 'global'
                        ? 'badge-ghost text-base-content/35'
                        : 'badge-warning text-warning-content'">
                      {{ w.source === 'global' ? 'Global' : 'Proj' }}
                    </span>
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
                <span v-if="!selectedSkills.length && !selectedWorkflows.length"
                  class="text-[11px] font-mono text-base-content/30">nothing selected</span>
              </div>
              <button class="btn btn-xs btn-primary font-mono"
                :disabled="exporting || (selectedSkills.length === 0 && selectedWorkflows.length === 0)"
                @click="exportBundle">
                <span v-if="exporting" class="loading loading-spinner loading-xs"></span>
                <span v-else class="text-xs">↓</span>
                {{ exporting ? 'Exporting…' : 'Export' }}
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
        <div class="card bg-base-200 border border-base-300 shadow-sm">
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
              Import a <code class="font-mono bg-base-300/80 px-1 rounded">.a7bundle</code> file to add skills
              and workflows to this instance.
            </p>

            <!-- Hidden file input -->
            <input ref="fileInput" type="file" accept=".a7bundle,.json" class="hidden" @change="handleFileChange" />

            <!-- Drop zone -->
            <div class="border-2 border-dashed border-base-300 rounded-xl py-10 px-6 text-center cursor-pointer
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
                <div class="stat py-3 px-5">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Skills</div>
                  <div class="stat-value text-2xl text-primary">{{ bundlePreview.skillCount }}</div>
                </div>
                <div class="stat py-3 px-5">
                  <div class="stat-title text-[10px] font-mono uppercase tracking-widest">Workflows</div>
                  <div class="stat-value text-2xl text-secondary">{{ bundlePreview.workflowCount }}</div>
                </div>
              </div>

              <div class="flex items-center justify-between flex-wrap gap-3">
                <label class="flex items-center gap-2 cursor-pointer">
                  <input type="checkbox" class="toggle toggle-xs toggle-warning" v-model="overwrite" />
                  <span class="text-xs font-mono text-base-content/55">overwrite existing</span>
                </label>
                <button class="btn btn-sm btn-primary font-mono"
                  :disabled="importing" @click="importBundle">
                  <span v-if="importing" class="loading loading-spinner loading-xs"></span>
                  <span v-else class="text-xs">↑</span>
                  {{ importing ? 'Importing…' : 'Import' }}
                </button>
              </div>
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
        <div class="card bg-base-200 border border-base-300 lg:col-span-2">
          <div class="card-body p-5">
            <h3 class="font-bold font-mono text-[10px] uppercase tracking-widest text-base-content/40 mb-4">
              Bundle Format Reference
            </h3>
            <div class="grid grid-cols-1 md:grid-cols-3 gap-5">
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
                <span class="text-accent opacity-60 mt-0.5">◈</span>
                <div>
                  <p class="text-xs font-mono font-semibold text-base-content/70 mb-1">Container</p>
                  <p class="text-xs text-base-content/45 leading-relaxed">
                    JSON envelope with <code class="bg-base-300 px-0.5 rounded">version</code>,
                    <code class="bg-base-300 px-0.5 rounded">created_at</code>, and arrays of
                    <code class="bg-base-300 px-0.5 rounded">skills</code> and
                    <code class="bg-base-300 px-0.5 rounded">workflows</code> — each with filename, content, and sha256.
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

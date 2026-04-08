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
  selectedWorkflows.value = allWorkflowsSelected.value ? [] : [...workflows.value]
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
    selectedWorkflows.value = [...wf]
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
    <div class="p-4 border-b border-base-300 bg-base-200 flex items-center justify-between">
      <div>
        <h2 class="text-lg font-bold">Sharing</h2>
        <p class="text-xs text-base-content/50 mt-0.5">Export and import skill &amp; workflow bundles</p>
      </div>
    </div>

    <div class="flex-1 overflow-auto p-6 space-y-8 max-w-3xl">

      <!-- ── Export Panel ────────────────────────────────────────────── -->
      <section>
        <h3 class="text-sm font-bold uppercase tracking-wider text-base-content/60 mb-4">Export Bundle</h3>
        <p class="text-xs text-base-content/50 mb-4">
          Bundle selected skills and workflows into a single <code class="font-mono bg-base-300 px-1 rounded">.a7bundle</code> file
          that can be imported on any agent007 instance.
        </p>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
          <!-- Skills picker -->
          <div class="card bg-base-200 border border-base-300 p-4">
            <div class="flex items-center justify-between mb-3">
              <span class="text-xs font-bold uppercase tracking-wider text-base-content/50">Skills</span>
              <label class="label cursor-pointer gap-2 p-0">
                <span class="text-xs text-base-content/40">All</span>
                <input type="checkbox" class="checkbox checkbox-xs" :checked="allSkillsSelected" @change="toggleAllSkills" />
              </label>
            </div>
            <div v-if="!skills.length" class="text-xs text-base-content/30 italic">No skills installed</div>
            <div class="space-y-1 max-h-48 overflow-y-auto">
              <label
                v-for="s in skills"
                :key="s.trigger"
                class="flex items-center gap-2 cursor-pointer hover:bg-base-300/50 rounded px-1 py-0.5"
              >
                <input
                  type="checkbox"
                  class="checkbox checkbox-xs"
                  :value="s.trigger.replace(/^\//, '')"
                  v-model="selectedSkills"
                />
                <span class="text-xs font-mono truncate">{{ s.trigger }}</span>
                <span class="text-xs text-base-content/40 truncate">{{ s.name }}</span>
              </label>
            </div>
          </div>

          <!-- Workflows picker -->
          <div class="card bg-base-200 border border-base-300 p-4">
            <div class="flex items-center justify-between mb-3">
              <span class="text-xs font-bold uppercase tracking-wider text-base-content/50">Workflows</span>
              <label class="label cursor-pointer gap-2 p-0">
                <span class="text-xs text-base-content/40">All</span>
                <input type="checkbox" class="checkbox checkbox-xs" :checked="allWorkflowsSelected" @change="toggleAllWorkflows" />
              </label>
            </div>
            <div v-if="!workflows.length" class="text-xs text-base-content/30 italic">No workflows saved</div>
            <div class="space-y-1 max-h-48 overflow-y-auto">
              <label
                v-for="w in workflows"
                :key="w"
                class="flex items-center gap-2 cursor-pointer hover:bg-base-300/50 rounded px-1 py-0.5"
              >
                <input
                  type="checkbox"
                  class="checkbox checkbox-xs"
                  :value="w"
                  v-model="selectedWorkflows"
                />
                <span class="text-xs font-mono truncate">{{ w }}</span>
              </label>
            </div>
          </div>
        </div>

        <div class="mt-4 flex items-center gap-3">
          <button
            class="btn btn-sm btn-primary gap-1.5"
            :class="{ 'loading loading-spinner': exporting }"
            :disabled="exporting || (selectedSkills.length === 0 && selectedWorkflows.length === 0)"
            @click="exportBundle"
          >
            <span v-if="!exporting">↓ Export Bundle</span>
            <span v-else>Exporting…</span>
          </button>
          <span v-if="selectedSkills.length || selectedWorkflows.length" class="text-xs text-base-content/40">
            {{ selectedSkills.length }} skill{{ selectedSkills.length !== 1 ? 's' : '' }},
            {{ selectedWorkflows.length }} workflow{{ selectedWorkflows.length !== 1 ? 's' : '' }}
          </span>
        </div>

        <div v-if="exportStatus" class="mt-3 alert alert-sm" :class="{
          'alert-success': exportStatus.type === 'success',
          'alert-error': exportStatus.type === 'error',
        }">
          <span class="text-sm">{{ exportStatus.message }}</span>
        </div>
      </section>

      <div class="divider"></div>

      <!-- ── Import Panel ────────────────────────────────────────────── -->
      <section>
        <h3 class="text-sm font-bold uppercase tracking-wider text-base-content/60 mb-4">Import Bundle</h3>
        <p class="text-xs text-base-content/50 mb-4">
          Import a <code class="font-mono bg-base-300 px-1 rounded">.a7bundle</code> file to add skills and workflows to this instance.
        </p>

        <!-- File picker -->
        <input
          ref="fileInput"
          type="file"
          accept=".a7bundle,.json"
          class="hidden"
          @change="handleFileChange"
        />

        <div
          class="border-2 border-dashed border-base-300 rounded-xl p-8 text-center cursor-pointer hover:border-primary/50 hover:bg-primary/5 transition-colors"
          @click="openFilePicker"
        >
          <div class="text-3xl mb-2 select-none">📦</div>
          <p class="text-sm text-base-content/60">Click to select a <span class="font-mono">.a7bundle</span> file</p>
          <p class="text-xs text-base-content/30 mt-1">or drag it here (browser permitting)</p>
        </div>

        <!-- Parse error -->
        <div v-if="parseError" class="mt-3 alert alert-error alert-sm">
          <span class="text-sm">{{ parseError }}</span>
        </div>

        <!-- Preview -->
        <div v-if="bundlePreview" class="mt-4 card bg-base-200 border border-base-300 p-4">
          <p class="text-xs font-bold uppercase tracking-wider text-base-content/50 mb-2">Bundle Preview</p>
          <div class="flex gap-6">
            <div class="text-center">
              <div class="text-2xl font-bold text-primary">{{ bundlePreview.skillCount }}</div>
              <div class="text-xs text-base-content/50">Skill{{ bundlePreview.skillCount !== 1 ? 's' : '' }}</div>
            </div>
            <div class="text-center">
              <div class="text-2xl font-bold text-secondary">{{ bundlePreview.workflowCount }}</div>
              <div class="text-xs text-base-content/50">Workflow{{ bundlePreview.workflowCount !== 1 ? 's' : '' }}</div>
            </div>
          </div>

          <!-- Overwrite toggle + Import button -->
          <div class="mt-4 flex items-center gap-4 flex-wrap">
            <label class="label cursor-pointer gap-2 p-0">
              <input type="checkbox" class="toggle toggle-xs" v-model="overwrite" />
              <span class="text-xs text-base-content/60">Overwrite existing</span>
            </label>
            <button
              class="btn btn-sm btn-primary gap-1.5"
              :class="{ 'loading loading-spinner': importing }"
              :disabled="importing"
              @click="importBundle"
            >
              <span v-if="!importing">↑ Import</span>
              <span v-else>Importing…</span>
            </button>
          </div>
        </div>

        <!-- Import result -->
        <div v-if="importStatus" class="mt-3 alert alert-sm" :class="{
          'alert-success': importStatus.type === 'success',
          'alert-error': importStatus.type === 'error',
        }">
          <span class="text-sm">{{ importStatus.message }}</span>
        </div>
      </section>

    </div>
  </div>
</template>

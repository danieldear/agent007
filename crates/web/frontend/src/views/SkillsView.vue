<script setup>
import { ref, computed, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api, loading } = useApi()
const skills = ref([])
const registry = ref([])
const activeTab = ref('installed')
const showForm = ref(false)
const editingTrigger = ref(null) // null = creating, string = editing existing
const importUrl = ref('')
const importStatus = ref(null)
const searchQuery = ref('')

const DEFAULT_TEMPLATE = `You are a helpful AI assistant.

# Task
{{args}}

# Instructions
1. Analyze the input carefully.
2. Provide a clear, structured response.
3. Include examples where helpful.

Use {{task}} for workflow context when running inside a workflow.`

const form = ref({
  name: '',
  trigger: '/',
  description: '',
  model: 'codex',
  template: '',
})

onMounted(loadSkills)

async function loadSkills() {
  const data = await api.listSkills()
  if (data) skills.value = data
}

async function loadRegistry() {
  if (registry.value.length) return
  try {
    const data = await api.getRegistry()
    if (data) registry.value = Array.isArray(data) ? data : []
  } catch {
    registry.value = []
  }
}

function switchTab(tab) {
  activeTab.value = tab
  if (tab === 'browse') loadRegistry()
}

const categoryOrder = ['dev', 'code', 'project', 'meta', 'custom']
const categoryLabels = {
  dev: 'Development',
  code: 'Code Quality',
  project: 'Project Management',
  meta: 'Meta / Tooling',
  custom: 'Custom',
}

const grouped = computed(() => {
  const groups = {}
  for (const s of skills.value) {
    const cat = s.category || inferCategory(s.trigger) || 'custom'
    if (!groups[cat]) groups[cat] = []
    groups[cat].push(s)
  }
  return groups
})

const sortedCategories = computed(() => {
  return categoryOrder.filter(c => grouped.value[c]?.length)
})

function inferCategory(trigger) {
  if (!trigger) return 'custom'
  const t = trigger.replace(/^\//, '')
  if (t.startsWith('dev-')) return 'dev'
  if (t.startsWith('code-')) return 'code'
  if (t.startsWith('project-')) return 'project'
  if (t.startsWith('meta-')) return 'meta'
  return 'custom'
}

const filteredRegistry = computed(() => {
  if (!searchQuery.value) return registry.value
  const q = searchQuery.value.toLowerCase()
  return registry.value.filter(s =>
    (s.name || '').toLowerCase().includes(q) ||
    (s.trigger || '').toLowerCase().includes(q) ||
    (s.description || '').toLowerCase().includes(q) ||
    (s.category || '').toLowerCase().includes(q)
  )
})

const installedTriggers = computed(() => new Set(skills.value.map(s => s.trigger)))

function openCreate() {
  editingTrigger.value = null
  form.value = { name: '', trigger: '/', description: '', model: 'codex', template: DEFAULT_TEMPLATE }
  showForm.value = true
}

async function openEdit(skill) {
  editingTrigger.value = skill.trigger
  // Strip leading "/" for API call
  const triggerParam = skill.trigger.replace(/^\//, '')
  try {
    const detail = await api.getSkill(triggerParam)
    if (detail) {
      form.value = {
        name: detail.name || skill.name || '',
        trigger: detail.trigger || skill.trigger || '/',
        description: detail.description || skill.description || '',
        model: detail.model || 'codex',
        template: detail.template || '',
      }
    } else {
      form.value = {
        name: skill.name || '',
        trigger: skill.trigger || '/',
        description: skill.description || '',
        model: 'codex',
        template: '',
      }
    }
  } catch {
    form.value = {
      name: skill.name || '',
      trigger: skill.trigger || '/',
      description: skill.description || '',
      model: 'codex',
      template: '',
    }
  }
  showForm.value = true
}

async function saveSkill() {
  await api.saveSkill(form.value)
  showForm.value = false
  editingTrigger.value = null
  await loadSkills()
}

async function installFromRegistry(item) {
  if (!item.url) return
  importStatus.value = { type: 'loading', message: `Installing ${item.name}...` }
  try {
    await api.importSkill(item.url)
    importStatus.value = { type: 'success', message: `${item.name} installed!` }
    await loadSkills()
    setTimeout(() => importStatus.value = null, 3000)
  } catch (e) {
    importStatus.value = { type: 'error', message: e.message }
  }
}

async function importFromUrl() {
  if (!importUrl.value.trim()) return
  importStatus.value = { type: 'loading', message: 'Importing...' }
  try {
    const result = await api.importSkill(importUrl.value.trim())
    importStatus.value = { type: 'success', message: `Imported ${result?.trigger || 'skill'} successfully!` }
    importUrl.value = ''
    await loadSkills()
    setTimeout(() => importStatus.value = null, 3000)
  } catch (e) {
    importStatus.value = { type: 'error', message: e.message }
  }
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="p-4 border-b border-base-300 bg-base-200 flex items-center justify-between">
      <h2 class="text-lg font-bold">Skills</h2>
      <button class="btn btn-sm btn-primary" @click="openCreate">+ New Skill</button>
    </div>

    <!-- Tabs -->
    <div class="tabs tabs-bordered px-4 pt-2 bg-base-200 border-b border-base-300">
      <a class="tab" :class="{ 'tab-active': activeTab === 'installed' }" @click="switchTab('installed')">
        Installed
        <span class="badge badge-sm ml-1.5">{{ skills.length }}</span>
      </a>
      <a class="tab" :class="{ 'tab-active': activeTab === 'browse' }" @click="switchTab('browse')">Browse Registry</a>
      <a class="tab" :class="{ 'tab-active': activeTab === 'import' }" @click="switchTab('import')">Import</a>
    </div>

    <div class="flex-1 overflow-auto p-4">

      <!-- Tab: Installed -->
      <div v-if="activeTab === 'installed'" class="space-y-6">
        <template v-for="cat in sortedCategories" :key="cat">
          <div>
            <h3 class="text-sm font-bold text-base-content/60 uppercase tracking-wider mb-3 flex items-center gap-2">
              <span v-if="cat === 'dev'" class="text-blue-400">&#9672;</span>
              <span v-else-if="cat === 'code'" class="text-green-400">&#9672;</span>
              <span v-else-if="cat === 'project'" class="text-amber-400">&#9672;</span>
              <span v-else-if="cat === 'meta'" class="text-purple-400">&#9672;</span>
              <span v-else class="text-base-content/30">&#9672;</span>
              {{ categoryLabels[cat] || cat }}
            </h3>
            <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
              <div
                v-for="s in grouped[cat]"
                :key="s.trigger"
                class="card bg-base-200 border border-base-300 p-4 hover:border-primary/50 transition-colors cursor-pointer group"
                @click="openEdit(s)"
                :title="'Click to view / edit prompt'"
              >
                <div class="flex items-start gap-3">
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-2">
                      <div class="font-medium text-sm">{{ s.name }}</div>
                      <span class="badge badge-xs badge-primary font-mono">{{ s.trigger }}</span>
                    </div>
                    <div class="text-xs text-base-content/50 mt-1">{{ s.description }}</div>
                  </div>
                  <!-- Edit hint shown on hover -->
                  <div class="opacity-0 group-hover:opacity-100 transition-opacity text-base-content/40 text-xs flex items-center gap-1 shrink-0">
                    <span>✏️</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </template>

        <div v-if="!skills.length && !loading" class="text-center text-base-content/40 py-12">
          <p class="text-lg mb-2">No skills installed yet</p>
          <p class="text-sm">Run <code class="font-mono bg-base-300 px-1.5 py-0.5 rounded">agent007 init</code> to install built-in skills, or import from the registry.</p>
        </div>
      </div>

      <!-- Tab: Browse Registry -->
      <div v-if="activeTab === 'browse'" class="space-y-4">
        <div class="form-control">
          <input
            v-model="searchQuery"
            class="input input-sm input-bordered w-full max-w-md"
            placeholder="Search registry skills..."
          />
        </div>

        <div v-if="!registry.length && !loading" class="text-center text-base-content/40 py-12">
          <p class="text-lg mb-2">Registry is empty or unavailable</p>
          <p class="text-sm">The community registry at GitHub is not yet populated. Use the Import tab to add skills from any URL.</p>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          <div
            v-for="item in filteredRegistry"
            :key="item.trigger || item.name"
            class="card bg-base-200 border border-base-300 p-4"
          >
            <div class="flex items-start justify-between">
              <div class="flex-1 min-w-0">
                <div class="font-medium text-sm">{{ item.name }}</div>
                <div class="text-xs text-base-content/50 mt-0.5">{{ item.description }}</div>
                <div class="flex gap-1 mt-2">
                  <span v-if="item.trigger" class="badge badge-xs font-mono">{{ item.trigger }}</span>
                  <span v-if="item.category" class="badge badge-xs badge-ghost">{{ item.category }}</span>
                </div>
              </div>
              <button
                v-if="installedTriggers.has(item.trigger)"
                class="btn btn-xs btn-disabled"
                disabled
              >Installed</button>
              <button
                v-else
                class="btn btn-xs btn-primary"
                @click="installFromRegistry(item)"
              >Install</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Tab: Import -->
      <div v-if="activeTab === 'import'" class="space-y-6 max-w-2xl">
        <div>
          <h3 class="text-sm font-bold mb-2">Import from URL</h3>
          <p class="text-xs text-base-content/50 mb-3">
            Paste a GitHub URL to a skill <code>.md</code> file. Supports raw URLs and blob URLs (auto-converted).
          </p>
          <div class="flex gap-2">
            <input
              v-model="importUrl"
              class="input input-sm input-bordered flex-1 font-mono"
              placeholder="https://github.com/user/repo/blob/main/skills/my-skill.md"
            />
            <button class="btn btn-sm btn-primary" @click="importFromUrl" :disabled="!importUrl.trim()">
              Import
            </button>
          </div>
        </div>

        <div v-if="importStatus" class="alert alert-sm" :class="{
          'alert-info': importStatus.type === 'loading',
          'alert-success': importStatus.type === 'success',
          'alert-error': importStatus.type === 'error',
        }">
          <span>{{ importStatus.message }}</span>
        </div>

        <div class="divider text-xs text-base-content/30">SUPPORTED FORMATS</div>
        <div class="text-xs text-base-content/50 space-y-2">
          <p>Skill files must be Markdown with YAML frontmatter:</p>
          <pre class="bg-base-300 p-3 rounded-lg font-mono text-xs overflow-x-auto">---
name: My Skill
trigger: /my-skill
description: What this skill does
model: codex
category: dev
---
Your prompt template here.
Use &#123;&#123;args&#125;&#125; for input and &#123;&#123;task&#125;&#125; for workflow context.</pre>
        </div>
      </div>
    </div>

    <!-- Create / Edit modal -->
    <dialog :open="showForm" class="modal" :class="{ 'modal-open': showForm }">
      <div class="modal-box max-w-2xl bg-base-200">
        <h3 class="font-bold text-lg">
          {{ editingTrigger ? `Edit Skill — ${editingTrigger}` : 'Create Skill' }}
        </h3>

        <div class="mt-4 space-y-3">
          <div class="grid grid-cols-2 gap-3">
            <div class="form-control">
              <label class="label"><span class="label-text text-xs">Name</span></label>
              <input v-model="form.name" class="input input-sm input-bordered" :placeholder="editingTrigger ? '' : 'PR Reviewer'" />
            </div>
            <div class="form-control">
              <label class="label"><span class="label-text text-xs">Trigger</span></label>
              <input
                v-model="form.trigger"
                class="input input-sm input-bordered font-mono"
                placeholder="/my-skill"
                :disabled="!!editingTrigger"
                :class="{ 'opacity-60': !!editingTrigger }"
              />
            </div>
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Description</span></label>
            <input v-model="form.description" class="input input-sm input-bordered" placeholder="What this skill does" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Model</span></label>
            <select v-model="form.model" class="select select-sm select-bordered">
              <option>codex</option>
              <option>gpt-5.3-codex</option>
              <option>claude</option>
              <option>claude-sonnet-4-6</option>
              <option>ollama</option>
            </select>
          </div>
          <div class="form-control">
            <label class="label">
              <span class="label-text text-xs">Prompt Template</span>
              <span class="label-text-alt text-xs text-base-content/40">Use <code class="font-mono bg-base-300 px-1 rounded">&#123;&#123;args&#125;&#125;</code> for user input, <code class="font-mono bg-base-300 px-1 rounded">&#123;&#123;task&#125;&#125;</code> for workflow context</span>
            </label>
            <textarea
              v-model="form.template"
              class="textarea textarea-bordered text-sm font-mono h-64 resize-y"
              :placeholder="DEFAULT_TEMPLATE"
            />
          </div>
        </div>

        <div class="modal-action">
          <button class="btn btn-sm btn-ghost" @click="showForm = false; editingTrigger = null">Cancel</button>
          <button class="btn btn-sm btn-primary" @click="saveSkill" :disabled="!form.name || !form.trigger || !form.template">
            {{ editingTrigger ? 'Save Changes' : 'Save Skill' }}
          </button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showForm = false; editingTrigger = null">close</button></form>
    </dialog>
  </div>
</template>

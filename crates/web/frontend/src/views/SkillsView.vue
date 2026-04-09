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
const toast = ref(null)
const promotingTrigger = ref(null)

let toastTimer = null
function showToast(message, type = 'success') {
  clearTimeout(toastTimer)
  toast.value = { message, type }
  toastTimer = setTimeout(() => { toast.value = null }, 3500)
}

async function promoteSkill(skill) {
  const trigger = skill.trigger.replace(/^\//, '')
  promotingTrigger.value = trigger
  try {
    const { ok, status, body } = await api.promoteSkill(trigger)
    if (status === 409) {
      showToast('Already exists globally', 'info')
    } else if (ok) {
      showToast('Promoted to global!', 'success')
    } else {
      showToast(body?.error || `Error ${status}`, 'error')
    }
  } catch (e) {
    showToast(e.message || 'Promote failed', 'error')
  } finally {
    promotingTrigger.value = null
  }
}

const DEFAULT_TEMPLATE = `You are a helpful AI assistant.

# Task
{{args}}

# Instructions
1. Analyze the input carefully.
2. Provide a clear, structured response.
3. Include examples where helpful.

---
Prior context from memory (use this to avoid repeating analysis):
{{rag_context}}

Project notes and decisions:
{{memory.project}}`

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
    <!-- Toast -->
    <div v-if="toast" class="toast toast-top toast-end z-50 pointer-events-none">
      <div class="alert alert-sm shadow-lg" :class="{
        'alert-success': toast.type === 'success',
        'alert-info': toast.type === 'info',
        'alert-error': toast.type === 'error',
      }">
        <span class="text-sm">{{ toast.message }}</span>
      </div>
    </div>
    <div class="px-5 py-3.5 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40">Skills</span>
      <button class="btn btn-sm btn-primary font-mono text-xs px-4" @click="openCreate">+ new</button>
    </div>

    <!-- Tabs -->
    <div class="flex items-center gap-0 px-5 bg-base-200 border-b border-base-300">
      <button
        v-for="tab in ['installed', 'browse', 'import']"
        :key="tab"
        class="px-4 py-2.5 text-[11px] font-mono uppercase tracking-widest border-b-2 transition-colors"
        :class="activeTab === tab
          ? 'border-primary text-primary'
          : 'border-transparent text-base-content/40 hover:text-base-content/70'"
        @click="switchTab(tab)"
      >
        {{ tab }}<span v-if="tab === 'installed'" class="ml-1.5 text-[10px] text-base-content/30">({{ skills.length }})</span>
      </button>
    </div>

    <div class="flex-1 overflow-auto p-4">

      <!-- Tab: Installed -->
      <div v-if="activeTab === 'installed'" class="space-y-6">
        <template v-for="cat in sortedCategories" :key="cat">
          <div>
            <div class="flex items-center gap-2 mb-3">
              <span
                class="w-0.5 h-4 rounded-full shrink-0"
                :class="{
                  'bg-blue-400': cat === 'dev',
                  'bg-green-400': cat === 'code',
                  'bg-amber-400': cat === 'project',
                  'bg-purple-400': cat === 'meta',
                  'bg-base-content/30': !['dev','code','project','meta'].includes(cat),
                }"
              ></span>
              <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40">{{ categoryLabels[cat] || cat }}</span>
              <span class="text-[10px] font-mono text-base-content/20">({{ grouped[cat].length }})</span>
              <div class="flex-1 h-px bg-base-content/8"></div>
            </div>
            <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
              <div
                v-for="s in grouped[cat]"
                :key="s.trigger"
                class="bg-base-200 border border-base-300 rounded-lg p-4 hover:border-primary/40 transition-colors cursor-pointer group border-l-2"
                :class="{
                  'border-l-blue-400/50': cat === 'dev',
                  'border-l-green-400/50': cat === 'code',
                  'border-l-amber-400/50': cat === 'project',
                  'border-l-purple-400/50': cat === 'meta',
                  'border-l-base-content/20': !['dev','code','project','meta'].includes(cat),
                }"
                @click="openEdit(s)"
                :title="'Click to view / edit prompt'"
              >
                <div class="flex items-start gap-3">
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-2 flex-wrap">
                      <div class="font-mono text-sm font-semibold">{{ s.name }}</div>
                      <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-primary/30 text-primary/70">{{ s.trigger }}</span>
                      <!-- Source badge — always visible -->
                      <span
                        v-if="s.source === 'global'"
                        class="text-[9px] font-mono font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border border-success/30 text-success/70 bg-success/5"
                      >global</span>
                      <span
                        v-else
                        class="text-[9px] font-mono font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border border-warning/30 text-warning/70 bg-warning/5"
                      >proj</span>
                    </div>
                    <div class="text-[11px] font-mono text-base-content/45 mt-1.5 leading-relaxed">{{ s.description }}</div>
                  </div>
                  <div class="flex flex-col items-end gap-1 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button
                      v-if="s.source === 'project'"
                      class="btn btn-xs btn-ghost font-mono text-[10px] px-1.5 h-6 min-h-0 leading-none text-warning/70 hover:text-success"
                      :class="{ 'loading loading-spinner': promotingTrigger === s.trigger.replace(/^\//, '') }"
                      :disabled="promotingTrigger === s.trigger.replace(/^\//, '')"
                      title="Promote to global ~/.agent007/skills/"
                      @click.stop="promoteSkill(s)"
                    >↑ promote</button>
                    <span class="text-base-content/30 font-mono text-[10px]">◦ edit</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </template>

        <div v-if="!skills.length && !loading" class="text-center py-12">
          <div class="text-3xl mb-3 text-base-content/10">⚡</div>
          <p class="text-sm font-mono text-base-content/40">no skills installed</p>
          <p class="text-xs font-mono text-base-content/25 mt-1">run <code class="bg-base-300 px-1 rounded">agent007 init</code> or import from the registry</p>
        </div>
      </div>

      <!-- Tab: Browse Registry -->
      <div v-if="activeTab === 'browse'" class="space-y-4">
        <div class="form-control">
          <input
            v-model="searchQuery"
            class="input input-sm input-bordered w-full max-w-md font-mono"
            placeholder="search skills..."
          />
        </div>

        <div v-if="!registry.length && !loading" class="text-center py-12">
          <div class="text-3xl mb-3 text-base-content/10">⬡</div>
          <p class="text-sm font-mono text-base-content/40">registry unavailable</p>
          <p class="text-xs font-mono text-base-content/25 mt-1">use the import tab to add skills from any URL</p>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          <div
            v-for="item in filteredRegistry"
            :key="item.trigger || item.name"
            class="bg-base-200 border border-base-300 rounded-lg p-4 border-l-2 border-l-base-content/15"
          >
            <div class="flex items-start justify-between">
              <div class="flex-1 min-w-0">
                <div class="font-mono text-sm font-semibold">{{ item.name }}</div>
                <div class="text-[11px] font-mono text-base-content/45 mt-1">{{ item.description }}</div>
                <div class="flex gap-1.5 mt-2">
                  <span v-if="item.trigger" class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-primary/30 text-primary/70">{{ item.trigger }}</span>
                  <span v-if="item.category" class="text-[10px] font-mono text-base-content/30">{{ item.category }}</span>
                </div>
              </div>
              <button
                v-if="installedTriggers.has(item.trigger)"
                class="btn btn-xs btn-disabled font-mono text-[10px]"
                disabled
              >installed</button>
              <button
                v-else
                class="btn btn-xs btn-primary font-mono text-[10px]"
                @click="installFromRegistry(item)"
              >install</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Tab: Import -->
      <div v-if="activeTab === 'import'" class="space-y-6 max-w-2xl">
        <div>
          <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/40 mb-2">import from url</div>
          <p class="text-[11px] font-mono text-base-content/40 mb-3">
            Paste a GitHub URL to a skill <code class="bg-base-300 px-1 rounded">.md</code> file. Supports raw and blob URLs.
          </p>
          <div class="flex gap-2">
            <input
              v-model="importUrl"
              class="input input-sm input-bordered flex-1 font-mono"
              placeholder="https://github.com/user/repo/blob/main/skills/my-skill.md"
            />
            <button class="btn btn-sm btn-primary font-mono text-xs" @click="importFromUrl" :disabled="!importUrl.trim()">
              import
            </button>
          </div>
        </div>

        <div v-if="importStatus" class="alert alert-sm" :class="{
          'alert-info': importStatus.type === 'loading',
          'alert-success': importStatus.type === 'success',
          'alert-error': importStatus.type === 'error',
        }">
          <span class="font-mono text-xs">{{ importStatus.message }}</span>
        </div>

        <div class="flex items-center gap-2 my-4">
          <div class="flex-1 h-px bg-base-content/10"></div>
          <span class="text-[10px] font-mono text-base-content/25 uppercase tracking-widest">format</span>
          <div class="flex-1 h-px bg-base-content/10"></div>
        </div>
        <div class="text-[11px] font-mono text-base-content/45 space-y-2">
          <p>Skill files must be Markdown with YAML frontmatter:</p>
          <pre class="bg-base-300 p-3 rounded-lg overflow-x-auto text-[11px]">---
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
      <div class="modal-box max-w-2xl bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <!-- Header bar -->
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">
            {{ editingTrigger ? `edit · ${editingTrigger}` : 'create skill' }}
          </span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showForm = false; editingTrigger = null">✕</button>
        </div>

        <!-- Body -->
        <div class="p-5 space-y-4">
          <!-- Name + Trigger row -->
          <div class="grid grid-cols-5 gap-3">
            <div class="col-span-3">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">name</div>
              <input
                v-model="form.name"
                class="skill-input w-full"
                :placeholder="editingTrigger ? '' : 'PR Reviewer'"
              />
            </div>
            <div class="col-span-2">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">trigger</div>
              <input
                v-model="form.trigger"
                class="skill-input w-full font-mono"
                placeholder="/my-skill"
                :disabled="!!editingTrigger"
                :class="{ 'opacity-40 cursor-not-allowed': !!editingTrigger }"
              />
            </div>
          </div>

          <!-- Description -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">description</div>
            <input v-model="form.description" class="skill-input w-full" placeholder="What this skill does" />
          </div>

          <!-- Model -->
          <div>
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">model</div>
            <div class="flex gap-1.5 flex-wrap">
              <button
                v-for="m in ['codex', 'gpt-5.3-codex', 'claude', 'claude-sonnet-4-6', 'ollama']"
                :key="m"
                type="button"
                class="px-3 py-1 text-[11px] font-mono rounded border transition-colors"
                :class="form.model === m
                  ? 'bg-primary/15 border-primary/50 text-primary'
                  : 'bg-base-200 border-base-300 text-base-content/50 hover:border-base-content/30'"
                @click="form.model = m"
              >{{ m }}</button>
            </div>
          </div>

          <!-- Prompt template -->
          <div class="flex-1">
            <div class="flex items-center justify-between mb-1.5">
              <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest">prompt template</div>
              <div class="flex items-center gap-1.5 text-[10px] font-mono text-base-content/25">
                <code class="bg-base-200 px-1 py-0.5 rounded border border-base-300">&#123;&#123;args&#125;&#125;</code>
                <span>·</span>
                <code class="bg-base-200 px-1 py-0.5 rounded border border-base-300">&#123;&#123;task&#125;&#125;</code>
                <span>·</span>
                <code class="bg-base-200 px-1 py-0.5 rounded border border-base-300">&#123;&#123;memory.project&#125;&#125;</code>
                <span>·</span>
                <code class="bg-base-200 px-1 py-0.5 rounded border border-base-300">&#123;&#123;rag_context&#125;&#125;</code>
              </div>
            </div>
            <textarea
              v-model="form.template"
              class="w-full bg-base-200 border border-base-300 rounded text-[13px] font-mono text-base-content/80 p-3 h-64 resize-y focus:outline-none focus:border-primary/50 transition-colors leading-relaxed"
              :placeholder="DEFAULT_TEMPLATE"
            />
          </div>
        </div>

        <!-- Footer -->
        <div class="flex items-center justify-end gap-2 px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-ghost font-mono text-xs px-4" @click="showForm = false; editingTrigger = null">cancel</button>
          <button
            class="btn btn-sm btn-primary font-mono text-xs px-4"
            @click="saveSkill"
            :disabled="!form.name || !form.trigger || !form.template"
          >{{ editingTrigger ? 'save changes' : 'save skill' }}</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showForm = false; editingTrigger = null">close</button></form>
    </dialog>
  </div>
</template>

<style scoped>
.skill-input {
  background: oklch(var(--b3));
  border: 1px solid oklch(var(--bc) / 0.18);
  border-radius: 0.375rem;
  padding: 0.375rem 0.625rem;
  font-size: 0.8125rem;
  font-family: ui-monospace, 'Cascadia Code', monospace;
  color: oklch(var(--bc));
  outline: none;
  transition: border-color 0.15s;
}
.skill-input:focus {
  border-color: oklch(var(--p) / 0.5);
}
.skill-input:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

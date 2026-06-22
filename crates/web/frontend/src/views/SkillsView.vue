<script setup>
import { ref, computed, watch, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api, loading } = useApi()
const skills = ref([])
const registry = ref([])
const activeTab = ref('installed')
const showForm = ref(false)
const editingTrigger = ref(null) // null = creating, string = editing existing
const editingSource = ref(null)
const importUrl = ref('')
const browseStatus = ref(null)
const importStatus = ref(null)
const searchQuery = ref('')
const discoverQuery = ref('')
const discoverSourcesText = ref('')
const discoverResults = ref([])
const discoverWarnings = ref([])
const selectedDiscoverUrls = ref([])
const discovering = ref(false)
const bulkInstalling = ref(false)
const bulkConflictMode = ref('keep_existing')
const previewLoading = ref(false)
const previewError = ref(null)
const previewOpen = ref(false)
const previewData = ref(null)
const previewMode = ref('replace')
const previewAliasTrigger = ref('')
const toast = ref(null)
const promotingTrigger = ref(null)
const deletingTrigger = ref(null)

const DEFAULT_DISCOVER_SOURCES = [
  'https://github.com/openai/skills/tree/main/skills',
  'https://github.com/anthropics/skills/tree/main/skills',
  'https://github.com/vercel-labs/agent-skills/tree/main/skills',
]

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

async function deleteSkill(skill) {
  if (!confirm(`Delete skill "${skill.name}"?\nThis will remove the file from disk.`)) return
  const trigger = skill.trigger.replace(/^\//, '')
  deletingTrigger.value = trigger
  try {
    const { ok, body } = await api.deleteSkill(skill.trigger)
    if (ok) {
      showToast(`Deleted "${skill.name}"`, 'success')
      skills.value = skills.value.filter(s => s.trigger !== skill.trigger)
    } else {
      showToast(body?.error || 'Delete failed', 'error')
    }
  } catch (e) {
    showToast(e.message || 'Delete failed', 'error')
  } finally {
    deletingTrigger.value = null
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
  category: 'custom',
  template: '',
})

onMounted(async () => {
  loadDiscoverSources()
  await loadSkills()
})

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
  if (tab === 'browse') {
    importStatus.value = null
    loadRegistry()
  } else if (tab === 'import') {
    browseStatus.value = null
  }
}

function loadDiscoverSources() {
  try {
    const saved = localStorage.getItem('agent007.skillDiscoverSources')
    if (saved) {
      const parsed = JSON.parse(saved)
      if (Array.isArray(parsed) && parsed.length) {
        discoverSourcesText.value = parsed.join('\n')
        return
      }
    }
  } catch {}
  discoverSourcesText.value = DEFAULT_DISCOVER_SOURCES.join('\n')
}

function normalizedDiscoverSources() {
  return discoverSourcesText.value
    .split('\n')
    .map(v => v.trim())
    .filter(Boolean)
}

function saveDiscoverSources() {
  try {
    localStorage.setItem('agent007.skillDiscoverSources', JSON.stringify(normalizedDiscoverSources()))
  } catch {}
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
const selectedDiscoverResults = computed(() =>
  discoverResults.value.filter(item => selectedDiscoverUrls.value.includes(item.url))
)
const allDiscoverSelected = computed(() =>
  discoverResults.value.length > 0 && selectedDiscoverUrls.value.length === discoverResults.value.length
)

function assocTools(item) {
  return item?.associations?.tools || []
}

function assocScripts(item) {
  return item?.associations?.scripts || []
}

function hasAssociations(item) {
  return assocTools(item).length > 0 || assocScripts(item).length > 0
}

function previewAssociations(values, limit = 3) {
  return values.slice(0, limit)
}

function compactRef(value) {
  if (!value) return ''
  const normalized = String(value).replace(/^\.?\/*/, '')
  const parts = normalized.split('/').filter(Boolean)
  if (parts.length <= 2) return normalized
  return `${parts[0]}/…/${parts[parts.length - 1]}`
}

function skillScopeChips(item) {
  const variants = Array.isArray(item?.variants) ? item.variants : []
  if (!variants.length) {
    return [catalogSourceLabel(item?.source)]
  }
  return variants.map(v => `${catalogSourceLabel(v?.source)}${v?.version ? ` v${v.version}` : ''}`)
}

function catalogSourceLabel(source) {
  return ({
    project: 'proj',
    global: 'global',
    'project-pack': 'proj pack',
    'global-pack': 'global pack',
  })[source] || source || 'proj'
}

function isReadOnlyCatalogSource(source) {
  return source === 'project-pack' || source === 'global-pack'
}

function skillHasVersionDrift(item) {
  const variants = Array.isArray(item?.variants) ? item.variants : []
  const versions = [...new Set(variants.map(v => (v?.version || '').trim()).filter(Boolean))]
  return versions.length > 1
}

const categoryPrefixes = {
  dev: 'dev',
  code: 'code',
  project: 'project',
  meta: 'meta',
  custom: '',
}

function nameToSlug(name) {
  return name.trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

function autoTrigger(name, category) {
  const slug = nameToSlug(name)
  if (!slug) return '/'
  const prefix = categoryPrefixes[category] || ''
  return prefix ? `/${prefix}-${slug}` : `/${slug}`
}

function openCreate() {
  editingTrigger.value = null
  editingSource.value = null
  form.value = { name: '', trigger: '/', description: '', model: 'codex', category: 'custom', template: DEFAULT_TEMPLATE }
  showForm.value = true
}

// Auto-generate trigger from name + category when creating (not editing).
watch([() => form.value.name, () => form.value.category], ([name, category]) => {
  if (!editingTrigger.value) {
    form.value.trigger = autoTrigger(name, category)
  }
})

async function openEdit(skill) {
  editingTrigger.value = skill.trigger
  editingSource.value = skill.source || null
  // Strip leading "/" for API call
  const triggerParam = skill.trigger.replace(/^\//, '')
  try {
    const detail = await api.getSkill(triggerParam, skill.path || null)
    if (detail) {
      form.value = {
        name: detail.name || skill.name || '',
        trigger: detail.trigger || skill.trigger || '/',
        description: detail.description || skill.description || '',
        model: detail.model || 'codex',
        category: detail.category || skill.category || 'custom',
        template: detail.template || '',
      }
    } else {
      form.value = {
        name: skill.name || '',
        trigger: skill.trigger || '/',
        description: skill.description || '',
        model: 'codex',
        category: skill.category || 'custom',
        template: '',
      }
    }
  } catch {
    form.value = {
      name: skill.name || '',
      trigger: skill.trigger || '/',
      description: skill.description || '',
      model: 'codex',
      category: skill.category || 'custom',
      template: '',
    }
  }
  showForm.value = true
}

async function saveSkill() {
  await api.saveSkill(form.value)
  showForm.value = false
  editingTrigger.value = null
  editingSource.value = null
  await loadSkills()
}

async function installFromRegistry(item) {
  if (!item.url) return
  browseStatus.value = { type: 'loading', message: `Installing ${item.name}...` }
  try {
    await api.importSkill(item.url)
    browseStatus.value = { type: 'success', message: `${item.name} installed!` }
    await loadSkills()
    setTimeout(() => browseStatus.value = null, 3000)
  } catch (e) {
    browseStatus.value = { type: 'error', message: e.message }
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

async function runDiscoverSearch() {
  if (!discoverQuery.value.trim()) return
  const sources = normalizedDiscoverSources()
  saveDiscoverSources()
  discovering.value = true
  browseStatus.value = null
  discoverWarnings.value = []
  try {
    const result = await api.discoverSkills(discoverQuery.value.trim(), sources, 16)
    discoverResults.value = Array.isArray(result?.results) ? result.results : []
    selectedDiscoverUrls.value = selectedDiscoverUrls.value.filter(url =>
      discoverResults.value.some(item => item.url === url)
    )
    discoverWarnings.value = Array.isArray(result?.warnings) ? result.warnings : []
  } catch (e) {
    browseStatus.value = { type: 'error', message: e.message || 'Discover failed' }
    discoverResults.value = []
    selectedDiscoverUrls.value = []
    discoverWarnings.value = []
  } finally {
    discovering.value = false
  }
}

function toggleDiscoverSelection(item) {
  if (!item?.url) return
  const selected = new Set(selectedDiscoverUrls.value)
  if (selected.has(item.url)) selected.delete(item.url)
  else selected.add(item.url)
  selectedDiscoverUrls.value = Array.from(selected)
}

function toggleAllDiscoverResults() {
  selectedDiscoverUrls.value = allDiscoverSelected.value
    ? []
    : discoverResults.value.map(item => item.url).filter(Boolean)
}

async function installSelectedDiscoverResults() {
  const items = selectedDiscoverResults.value
  if (!items.length || bulkInstalling.value) return
  bulkInstalling.value = true
  browseStatus.value = { type: 'loading', message: `Installing ${items.length} selected skill(s)...` }
  const summary = { installed: 0, skipped: 0, failed: 0 }
  try {
    for (const item of items) {
      try {
        const opts = { conflict_action: bulkConflictMode.value }
        const result = await api.importSkill(item.url, opts)
        if (result?.skipped) summary.skipped += 1
        else summary.installed += 1
      } catch {
        summary.failed += 1
      }
    }
    await loadSkills()
    await runDiscoverSearch()
    selectedDiscoverUrls.value = []
    const parts = [`${summary.installed} installed`]
    if (summary.skipped) parts.push(`${summary.skipped} kept`)
    if (summary.failed) parts.push(`${summary.failed} failed`)
    browseStatus.value = {
      type: summary.failed ? 'error' : 'success',
      message: `Bulk import complete: ${parts.join(', ')}`,
    }
    setTimeout(() => browseStatus.value = null, 4000)
  } finally {
    bulkInstalling.value = false
  }
}

async function openPreview(url, installMode = 'preview') {
  previewOpen.value = true
  previewLoading.value = true
  previewError.value = null
  previewData.value = null
  previewMode.value = 'replace'
  previewAliasTrigger.value = ''
  try {
    previewData.value = await api.previewSkillImport(url)
    if (installMode === 'install' && previewData.value?.conflict) {
      previewMode.value = 'replace'
      previewAliasTrigger.value = `${previewData.value.trigger}-alt`
    }
  } catch (e) {
    previewError.value = e.message || 'Preview failed'
  } finally {
    previewLoading.value = false
  }
}

async function installFromPreview() {
  if (!previewData.value?.url) return
  browseStatus.value = { type: 'loading', message: `Installing ${previewData.value.name || previewData.value.trigger}...` }
  try {
    const opts = {}
    if (previewData.value?.conflict) {
      opts.conflict_action = previewMode.value
      if (previewMode.value === 'alias') {
        opts.alias_trigger = previewAliasTrigger.value.trim()
      }
    }
    const result = await api.importSkill(previewData.value.url, opts)
    if (result?.skipped) {
      browseStatus.value = { type: 'success', message: `Kept existing ${result.trigger}` }
    } else {
      browseStatus.value = { type: 'success', message: `Installed ${result?.trigger || previewData.value.trigger}` }
    }
    previewOpen.value = false
    previewData.value = null
    await loadSkills()
    if (discoverQuery.value.trim()) {
      await runDiscoverSearch()
    }
    setTimeout(() => browseStatus.value = null, 3000)
  } catch (e) {
    browseStatus.value = { type: 'error', message: e.message || 'Install failed' }
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

    <div class="flex-1 overflow-auto p-5 lg:p-6">

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
            <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3">
              <div
                v-for="s in grouped[cat]"
                :key="`${s.trigger}:${s.precedence_source || s.source || 'project'}`"
                class="bg-base-200 border border-base-300 rounded-lg p-4 hover:border-primary/40 transition-colors cursor-pointer group border-l-2 h-full"
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
                <div class="flex items-start gap-3 h-full">
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-2 flex-wrap">
                      <div class="font-mono text-sm font-semibold">{{ s.name }}</div>
                      <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-primary/30 text-primary/70">{{ s.trigger }}</span>
                      <span
                        v-for="chip in skillScopeChips(s)"
                        :key="`${s.trigger}:${chip}`"
                        class="text-[9px] font-mono font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border border-base-content/25 text-base-content/65 bg-base-content/5"
                      >{{ chip }}</span>
                      <span
                        v-if="skillHasVersionDrift(s)"
                        class="text-[9px] font-mono font-bold uppercase tracking-wider px-1.5 py-0.5 rounded border border-warning/35 text-warning/80 bg-warning/10"
                        title="Project and global variants have different versions"
                      >drift</span>
                    </div>
                    <div class="text-[11px] font-mono text-base-content/45 mt-1.5 leading-relaxed">{{ s.description }}</div>
                    <div v-if="hasAssociations(s)" class="mt-2.5 space-y-1.5">
                      <div v-if="assocTools(s).length" class="flex items-center gap-1 flex-wrap">
                        <span class="text-[9px] font-mono uppercase tracking-wider text-base-content/35">tools</span>
                        <span
                          v-for="tool in previewAssociations(assocTools(s), 3)"
                          :key="`${s.trigger}-tool-${tool}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-info/25 text-info/80 bg-info/5"
                          :title="tool"
                        >{{ compactRef(tool) }}</span>
                        <span v-if="assocTools(s).length > 3" class="text-[9px] font-mono text-base-content/35">
                          +{{ assocTools(s).length - 3 }}
                        </span>
                      </div>
                      <div v-if="assocScripts(s).length" class="flex items-center gap-1 flex-wrap">
                        <span class="text-[9px] font-mono uppercase tracking-wider text-base-content/35">scripts</span>
                        <span
                          v-for="script in previewAssociations(assocScripts(s), 3)"
                          :key="`${s.trigger}-script-${script}`"
                          class="text-[9px] font-mono px-1.5 py-0.5 rounded border border-accent/25 text-accent/80 bg-accent/5"
                          :title="script"
                        >{{ compactRef(script) }}</span>
                        <span v-if="assocScripts(s).length > 3" class="text-[9px] font-mono text-base-content/35">
                          +{{ assocScripts(s).length - 3 }}
                        </span>
                      </div>
                    </div>
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
                    <button
                      v-if="s.can_delete !== false"
                      class="btn btn-xs btn-ghost font-mono text-[10px] px-1.5 h-6 min-h-0 leading-none text-error/60 hover:text-error"
                      :class="{ 'loading loading-spinner': deletingTrigger === s.trigger.replace(/^\//, '') }"
                      :disabled="deletingTrigger === s.trigger.replace(/^\//, '')"
                      title="Delete this skill"
                      @click.stop="deleteSkill(s)"
                    >✕ del</button>
                    <span class="text-base-content/30 font-mono text-[10px]">◦ {{ isReadOnlyCatalogSource(s.source) ? 'override' : 'edit' }}</span>
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
        <div v-if="browseStatus" class="alert alert-sm" :class="{
          'alert-info': browseStatus.type === 'loading',
          'alert-success': browseStatus.type === 'success',
          'alert-error': browseStatus.type === 'error',
        }">
          <span class="font-mono text-xs">{{ browseStatus.message }}</span>
        </div>

        <div class="flex items-center gap-3 flex-wrap">
          <div class="form-control flex-1 min-w-48">
            <input
              v-model="searchQuery"
              class="input input-sm input-bordered w-full font-mono"
              placeholder="Filter curated registry…"
            />
          </div>
          <div class="flex items-center gap-1.5 text-xs font-mono text-base-content/40">
            <span>External search:</span>
            <button
              class="btn btn-xs btn-ghost font-mono border border-base-300"
              @click="$emit('navigate-external', 'extensions')"
              title="Browse npm MCP packages and GitHub extensions. Skill search here is currently curated registry only."
            >⊞ Extensions →</button>
          </div>
        </div>

        <div v-if="!registry.length && !loading" class="text-center py-12">
          <div class="text-3xl mb-3 text-base-content/10">⬡</div>
          <p class="text-sm font-mono text-base-content/40">local registry unavailable</p>
          <p class="text-xs font-mono text-base-content/25 mt-1">
            browse external packages in <strong class="font-mono">Extensions</strong>, or use the Import tab for a direct URL
          </p>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3">
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

        <div class="pt-4">
          <div class="flex items-center gap-2 mb-3">
            <div class="flex-1 h-px bg-base-content/10"></div>
            <span class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">discover across GitHub sources</span>
            <div class="flex-1 h-px bg-base-content/10"></div>
          </div>

          <div class="grid grid-cols-1 xl:grid-cols-[1.3fr_0.9fr] gap-3 mb-3">
            <div class="space-y-2">
              <input
                v-model="discoverQuery"
                class="input input-sm input-bordered w-full font-mono"
                placeholder="Search trigger, name, or description (for example: frontend-designer)"
                @keyup.enter="runDiscoverSearch"
              />
              <div class="text-[10px] font-mono text-base-content/35">Uses built-in defaults for OpenAI, Anthropic, and Vercel if you leave custom sources empty. Duplicate variants are preserved by source.</div>
            </div>
            <div class="space-y-2">
              <textarea
                v-model="discoverSourcesText"
                class="textarea textarea-bordered textarea-sm w-full font-mono min-h-[110px]"
                placeholder="One GitHub repo, tree, README, catalog, or package URL per line"
              ></textarea>
              <div class="flex justify-between items-center">
                <div class="text-[10px] font-mono text-base-content/35">Use repo root, tree, README, catalog, or package URLs to add community or internal sources on top of the defaults.</div>
                <button class="btn btn-sm btn-primary font-mono text-xs" :class="{ 'loading': discovering }" @click="runDiscoverSearch">
                  {{ discovering ? 'searching' : 'search' }}
                </button>
              </div>
            </div>
          </div>

          <div v-if="discoverWarnings.length" class="rounded-lg border border-warning/30 bg-warning/5 p-3 mb-3">
            <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-warning mb-1">source warnings</div>
            <div
              v-for="warning in discoverWarnings"
              :key="warning"
              class="text-[10px] font-mono text-base-content/50 break-words"
            >{{ warning }}</div>
          </div>

          <div v-if="!discoverResults.length && !discovering" class="text-[11px] font-mono text-base-content/35 py-2">
            No discovery results yet. Search defaults or add one or more source URLs, then preview before install.
          </div>

          <div
            v-if="discoverResults.length"
            class="flex flex-col lg:flex-row lg:items-center justify-between gap-3 rounded-lg border border-base-300 bg-base-200/50 px-3 py-2 mb-3"
          >
            <div class="flex items-center gap-3">
              <label class="label cursor-pointer gap-2 p-0">
                <input
                  type="checkbox"
                  class="checkbox checkbox-sm"
                  :checked="allDiscoverSelected"
                  @change="toggleAllDiscoverResults"
                />
                <span class="font-mono text-[11px] text-base-content/55">
                  {{ selectedDiscoverResults.length }} / {{ discoverResults.length }} selected
                </span>
              </label>
              <span class="text-[10px] font-mono text-base-content/35">Duplicates are preserved by source so you can choose the better variant.</span>
            </div>
            <div class="flex items-center gap-2">
              <select v-model="bulkConflictMode" class="select select-xs select-bordered font-mono text-[10px]">
                <option value="keep_existing">keep conflicts</option>
                <option value="replace">replace conflicts</option>
              </select>
              <button
                class="btn btn-xs btn-primary font-mono text-[10px]"
                :class="{ 'loading': bulkInstalling }"
                :disabled="!selectedDiscoverResults.length || bulkInstalling"
                @click="installSelectedDiscoverResults"
              >
                install selected
              </button>
            </div>
          </div>

          <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-3">
            <div
              v-for="item in discoverResults"
              :key="`${item.url}:${item.trigger}`"
              class="bg-base-200 border rounded-lg p-4 border-l-2 border-l-info/40 min-h-[190px]"
              :class="selectedDiscoverUrls.includes(item.url) ? 'border-info/70 shadow shadow-info/10' : 'border-base-300'"
            >
              <div class="flex items-start justify-between gap-3">
                <input
                  type="checkbox"
                  class="checkbox checkbox-sm mt-0.5"
                  :checked="selectedDiscoverUrls.includes(item.url)"
                  @change="toggleDiscoverSelection(item)"
                />
                <div class="flex-1 min-w-0">
                  <div class="font-mono text-sm font-semibold break-words leading-snug">{{ item.name }}</div>
                  <div class="text-[11px] font-mono text-base-content/45 mt-1 break-words leading-relaxed max-h-[3.6rem] overflow-hidden">{{ item.description }}</div>
                  <div class="flex gap-1.5 mt-2 flex-wrap">
                    <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-primary/30 text-primary/70">{{ item.trigger }}</span>
                    <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-base-content/20 text-base-content/55 break-all">{{ item.repo }}</span>
                    <span v-if="item.version" class="text-[10px] font-mono text-base-content/35">v{{ item.version }}</span>
                    <span v-if="item.installed" class="text-[10px] font-mono text-warning">conflict</span>
                    <span v-if="item.from_catalog" class="text-[10px] font-mono text-info">catalog</span>
                  </div>
                  <div class="mt-2 text-[10px] font-mono text-base-content/35 truncate" :title="item.path">{{ item.path }}</div>
                </div>
                <div class="flex flex-col gap-1 shrink-0">
                  <button class="btn btn-xs btn-ghost font-mono text-[10px]" @click="openPreview(item.url)">preview</button>
                  <button class="btn btn-xs btn-primary font-mono text-[10px]" @click="openPreview(item.url, 'install')">install</button>
                </div>
              </div>
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

    <dialog :open="previewOpen" class="modal" :class="{ 'modal-open': previewOpen }">
      <div class="modal-box max-w-3xl bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">
            {{ previewData?.trigger || 'skill preview' }}
          </span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="previewOpen = false">✕</button>
        </div>
        <div class="p-5 space-y-4">
          <div v-if="previewLoading" class="text-[11px] font-mono text-base-content/45">Loading preview…</div>
          <div v-else-if="previewError" class="alert alert-error alert-sm">
            <span class="font-mono text-xs">{{ previewError }}</span>
          </div>
          <template v-else-if="previewData">
            <div>
              <div class="flex items-center gap-2 flex-wrap">
                <div class="font-mono text-sm font-semibold">{{ previewData.name }}</div>
                <span class="text-[10px] font-mono px-1.5 py-0.5 rounded border border-primary/30 text-primary/70">{{ previewData.trigger }}</span>
                <span v-if="previewData.version" class="text-[10px] font-mono text-base-content/35">v{{ previewData.version }}</span>
                <span v-if="previewData.category" class="text-[10px] font-mono text-base-content/35">{{ previewData.category }}</span>
              </div>
              <div class="text-[11px] font-mono text-base-content/45 mt-1">{{ previewData.description }}</div>
            </div>

            <div v-if="previewData.conflict" class="border border-warning/30 bg-warning/5 rounded-lg p-3 space-y-2">
              <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-warning">trigger conflict</div>
              <div class="text-[11px] font-mono text-base-content/55">
                Installed variants already exist for <code class="bg-base-200 px-1 rounded">{{ previewData.conflict.trigger }}</code>.
              </div>
              <div class="space-y-1">
                <div
                  v-for="variant in previewData.conflict.variants"
                  :key="`${variant.path}:${variant.source}`"
                  class="text-[10px] font-mono text-base-content/45"
                >
                  {{ variant.source }} · {{ variant.name }}<span v-if="variant.version"> · v{{ variant.version }}</span>
                </div>
              </div>
              <div class="flex gap-3 flex-wrap pt-1">
                <label class="label cursor-pointer gap-2 font-mono text-[11px]"><input v-model="previewMode" type="radio" class="radio radio-xs" value="replace" /> replace</label>
                <label class="label cursor-pointer gap-2 font-mono text-[11px]"><input v-model="previewMode" type="radio" class="radio radio-xs" value="alias" /> alias</label>
                <label class="label cursor-pointer gap-2 font-mono text-[11px]"><input v-model="previewMode" type="radio" class="radio radio-xs" value="keep_existing" /> keep existing</label>
              </div>
              <input
                v-if="previewMode === 'alias'"
                v-model="previewAliasTrigger"
                class="input input-sm input-bordered w-full font-mono"
                placeholder="/frontend-designer-alt"
              />
            </div>

            <div v-if="previewData.package && previewData.files?.length" class="space-y-2">
              <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">package files</div>
              <div class="max-h-28 overflow-auto rounded border border-base-300 bg-base-200 p-2 text-[10px] font-mono text-base-content/50">
                <div v-for="file in previewData.files" :key="file">{{ file }}</div>
              </div>
            </div>

            <div class="space-y-2">
              <div class="text-[10px] font-mono font-bold uppercase tracking-widest text-base-content/35">prompt preview</div>
              <pre class="bg-base-200 border border-base-300 rounded-lg p-3 overflow-auto max-h-80 text-[11px] font-mono text-base-content/70 whitespace-pre-wrap">{{ previewData.body }}</pre>
            </div>
          </template>
        </div>
        <div class="flex items-center justify-end gap-2 px-5 py-3 bg-base-200 border-t border-base-300">
          <button class="btn btn-sm btn-ghost font-mono text-xs" @click="previewOpen = false">close</button>
          <button
            v-if="previewData && !previewLoading && !previewError"
            class="btn btn-sm btn-primary font-mono text-xs"
            @click="installFromPreview"
          >install</button>
        </div>
      </div>
    </dialog>

    <!-- Create / Edit modal -->
    <dialog :open="showForm" class="modal" :class="{ 'modal-open': showForm }">
      <div class="modal-box max-w-2xl bg-base-100 border border-base-300 rounded-lg p-0 overflow-hidden">
        <!-- Header bar -->
        <div class="flex items-center justify-between px-5 py-3 bg-base-200 border-b border-base-300">
          <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/50">
            {{ editingTrigger ? `${isReadOnlyCatalogSource(editingSource) ? 'override' : 'edit'} · ${editingTrigger}` : 'create skill' }}
          </span>
          <button class="btn btn-ghost btn-xs font-mono text-base-content/40 hover:text-base-content px-1" @click="showForm = false; editingTrigger = null">✕</button>
        </div>

        <!-- Body -->
        <div class="p-5 space-y-4">
          <div v-if="isReadOnlyCatalogSource(editingSource)" class="rounded border border-warning/30 bg-warning/5 px-3 py-2 text-[11px] font-mono text-warning/80">
            This skill comes from an enabled pack. Saving creates a writable project/global override; the verified pack file is not modified.
          </div>
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

          <!-- Category (only shown when creating, auto-drives trigger prefix) -->
          <div v-if="!editingTrigger">
            <div class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-1.5">category</div>
            <div class="flex gap-1.5 flex-wrap">
              <button
                v-for="cat in ['dev', 'code', 'project', 'meta', 'custom']"
                :key="cat"
                type="button"
                class="px-3 py-1 text-[11px] font-mono rounded border transition-colors"
                :class="form.category === cat
                  ? 'bg-primary/15 border-primary/50 text-primary'
                  : 'bg-base-200 border-base-300 text-base-content/50 hover:border-base-content/30'"
                @click="form.category = cat"
              >{{ cat }}</button>
            </div>
            <div class="mt-1 text-[10px] font-mono text-base-content/25">
              Auto-generates trigger prefix: <code class="bg-base-200 px-1 rounded">{{ form.trigger }}</code>
            </div>
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
          >{{ editingTrigger ? (isReadOnlyCatalogSource(editingSource) ? 'save override' : 'save changes') : 'save skill' }}</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showForm = false; editingTrigger = null">close</button></form>
    </dialog>
  </div>
</template>

<style scoped>
.skill-input {
  background: var(--color-base-300, var(--b3));
  border: 1px solid color-mix(in oklch, var(--color-base-content, var(--bc)) 20%, transparent);
  border-radius: 0.375rem;
  padding: 0.375rem 0.625rem;
  font-size: 0.8125rem;
  font-family: ui-monospace, 'Cascadia Code', monospace;
  color: var(--color-base-content, var(--bc));
  outline: none;
  transition: border-color 0.15s;
}
.skill-input:focus {
  border-color: color-mix(in oklch, var(--color-primary, var(--p)) 50%, transparent);
}
.skill-input:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api } = useApi()

// ── tab state ────────────────────────────────────────────────────────────
const activeTab = ref('browse')

// ── toast ────────────────────────────────────────────────────────────────
const toast = ref(null)
let toastTimer = null
function showToast(msg, type = 'success') {
  clearTimeout(toastTimer)
  toast.value = { msg, type }
  toastTimer = setTimeout(() => { toast.value = null }, 3500)
}

// ── browse tab — curated npm MCP packages ───────────────────────────────
const CURATED = [
  { pkg: '@modelcontextprotocol/server-filesystem',          desc: 'Read/write files and directories on local filesystem', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-github',              desc: 'GitHub issues, PRs, code search and repository management', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-brave-search',        desc: 'Web search powered by Brave Search API', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-puppeteer',           desc: 'Browser automation — navigate, screenshot, scrape pages', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-postgres',            desc: 'Query and manage PostgreSQL databases', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-slack',               desc: 'Read and send Slack messages and manage channels', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-memory',              desc: 'Persistent key-value memory store across sessions', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-fetch',               desc: 'HTTP fetch tool for retrieving URLs and APIs', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-sequential-thinking', desc: 'Structured step-by-step reasoning pipeline', kind: 'npm' },
  { pkg: '@modelcontextprotocol/server-aws-kb-retrieval',    desc: 'AWS Bedrock knowledge base retrieval and RAG', kind: 'npm' },
]

const browseSearch = ref('')
const filteredCurated = computed(() => {
  const q = browseSearch.value.trim().toLowerCase()
  if (!q) return CURATED
  return CURATED.filter(c =>
    c.pkg.toLowerCase().includes(q) || c.desc.toLowerCase().includes(q)
  )
})

function importFromBrowse(pkg) {
  form.value.source_kind = 'npm'
  form.value.source_ref  = pkg
  form.value.github_owner = ''
  form.value.github_repo  = ''
  form.value.github_ref   = ''
  bundle.value  = null
  activeTab.value = 'import'
}

// ── import tab ───────────────────────────────────────────────────────────
const form = ref({
  source_kind: 'npm',
  source_ref:  '',
  github_owner: '',
  github_repo:  '',
  github_ref:   '',
})

const sourceKindOptions = [
  { value: 'npm',    label: 'npm MCP package (via npx)' },
  { value: 'github', label: 'GitHub repository' },
  { value: 'local',  label: 'Local path' },
  { value: 'openapi',label: 'OpenAPI spec URL' },
  { value: 'claude', label: 'Claude Marketplace' },
]

function labelFor(kind) {
  switch (kind) {
    case 'npm':     return 'npm package name'
    case 'github':  return 'GitHub repo (owner/repo)'
    case 'local':   return 'local directory path'
    case 'openapi': return 'OpenAPI spec URL'
    case 'claude':  return 'GitHub repo or marketplace.json URL'
    default:        return 'source reference'
  }
}
function placeholderFor(kind) {
  switch (kind) {
    case 'npm':     return '@modelcontextprotocol/server-filesystem'
    case 'github':  return 'owner/repo  (or fill Owner + Repo below)'
    case 'local':   return '/path/to/extension-dir'
    case 'openapi': return 'https://api.example.com/openapi.json'
    case 'claude':  return 'https://github.com/owner/repo'
    default:        return ''
  }
}

const previewing = ref(false)
const installing = ref(false)
const bundle = ref(null)
const previewError = ref(null)

const compFlags = ref({ skills: true, tools: true, workflows: true, mcp: true, rag: true })

function buildPayload() {
  const p = {
    source_kind: form.value.source_kind,
    source_ref:  form.value.source_ref.trim(),
  }
  if (form.value.source_kind === 'github') {
    p.github_owner = form.value.github_owner.trim()
    p.github_repo  = form.value.github_repo.trim()
    if (form.value.github_ref.trim()) p.github_ref = form.value.github_ref.trim()
  }
  return p
}

async function preview() {
  previewError.value = null
  bundle.value = null
  previewing.value = true
  try {
    const payload = buildPayload()
    bundle.value = await api.previewExtension(payload)
  } catch (e) {
    previewError.value = e.message
  } finally {
    previewing.value = false
  }
}

async function install() {
  if (!bundle.value) return
  installing.value = true
  try {
    const payload = { ...buildPayload(), components: { ...compFlags.value } }
    const result = await api.installExtension(payload)
    showToast(`Installed "${result.extension}" successfully`)
    bundle.value = null
    form.value.source_ref = ''
    activeTab.value = 'installed'
    await loadInstalled()
  } catch (e) {
    showToast(e.message, 'error')
  } finally {
    installing.value = false
  }
}

function gradeClass(grade) {
  if (grade === 'A') return 'badge-success'
  if (grade === 'B') return 'badge-warning'
  return 'badge-error'
}

function gradeLabel(grade) {
  if (grade === 'A') return 'Grade A — fully compatible'
  if (grade === 'B') return 'Grade B — compatible, review recommended'
  return 'Grade C — partial compatibility'
}

function compSummary(ext) {
  const parts = []
  if (ext.skills)      parts.push(`${ext.skills} skill${ext.skills !== 1 ? 's' : ''}`)
  if (ext.tools)       parts.push(`${ext.tools} tool${ext.tools !== 1 ? 's' : ''}`)
  if (ext.workflows)   parts.push(`${ext.workflows} workflow${ext.workflows !== 1 ? 's' : ''}`)
  if (ext.mcp_servers) parts.push(`${ext.mcp_servers} MCP`)
  if (ext.rag_sources) parts.push(`${ext.rag_sources} RAG`)
  return parts.join(' · ') || '—'
}

// ── installed tab ────────────────────────────────────────────────────────
const installed = ref([])
const loadingInstalled = ref(false)

async function loadInstalled() {
  loadingInstalled.value = true
  try {
    const result = await api.listExtensions()
    installed.value = result.extensions || []
  } catch {}
  finally { loadingInstalled.value = false }
}

onMounted(loadInstalled)

function fmtDate(iso) {
  if (!iso) return ''
  try { return new Date(iso).toLocaleDateString() } catch { return iso }
}
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">

    <!-- Header -->
    <div class="px-6 pt-5 pb-0 border-b border-base-300/60 shrink-0">
      <div class="flex items-start justify-between mb-4">
        <div>
          <h2 class="text-sm font-bold font-mono uppercase tracking-widest text-base-content/60">Extensions</h2>
          <p class="text-xs text-base-content/40 mt-1 font-mono max-w-xl">
            Install bundles from external registries — each bundle can contain MCP servers, RAG sources, skills, tools, and workflows, all installed together in one step.
          </p>
        </div>
      </div>

      <!-- Tabs -->
      <div class="flex gap-0 border-b-0">
        <button
          v-for="t in ['browse', 'import', 'installed']"
          :key="t"
          class="px-4 py-2.5 text-xs font-mono border-b-2 transition-colors capitalize"
          :class="activeTab === t
            ? 'border-primary text-primary'
            : 'border-transparent text-base-content/40 hover:text-base-content/70'"
          @click="activeTab = t; if(t === 'installed') loadInstalled()"
        >
          {{ t }}
          <span v-if="t === 'installed' && installed.length" class="ml-1 text-[10px] text-base-content/30">({{ installed.length }})</span>
        </button>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto px-6 py-5">

      <!-- ── BROWSE TAB ── -->
      <div v-if="activeTab === 'browse'" class="space-y-4">

        <!-- search + info row -->
        <div class="flex items-center gap-3">
          <div class="flex-1 relative">
            <input
              v-model="browseSearch"
              type="text"
              class="input input-sm input-bordered w-full font-mono pr-8"
              placeholder="Search curated packages…"
            />
            <button v-if="browseSearch" class="absolute right-2 top-1/2 -translate-y-1/2 text-base-content/30 hover:text-base-content/70" @click="browseSearch = ''">✕</button>
          </div>
          <a
            class="btn btn-sm btn-ghost font-mono gap-1 text-xs text-base-content/50"
            href="https://github.com/modelcontextprotocol/servers"
            target="_blank"
            rel="noopener"
          >↗ MCP servers registry</a>
        </div>

        <!-- grade legend -->
        <div class="flex items-center gap-3 text-xs font-mono text-base-content/40">
          <span>Compat grade:</span>
          <span class="badge badge-success badge-xs">A</span><span>fully verified</span>
          <span class="badge badge-warning badge-xs">B</span><span>review recommended</span>
          <span class="badge badge-error badge-xs">C</span><span>partial support</span>
        </div>

        <!-- empty search -->
        <div v-if="!filteredCurated.length" class="text-center py-12 text-base-content/30 font-mono text-sm">
          No packages match "{{ browseSearch }}"
        </div>

        <!-- grid -->
        <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3">
          <div
            v-for="item in filteredCurated"
            :key="item.pkg"
            class="bg-base-200 border border-base-300/60 rounded-lg p-4 flex flex-col gap-2.5 hover:border-primary/30 transition-colors"
          >
            <div class="flex items-start justify-between gap-2">
              <code class="text-xs text-primary/90 font-mono break-all leading-snug flex-1">{{ item.pkg }}</code>
              <span class="badge badge-warning badge-xs shrink-0 font-mono" title="Grade B — review recommended">B</span>
            </div>
            <p class="text-xs text-base-content/55 leading-relaxed flex-1">{{ item.desc }}</p>
            <div class="flex items-center justify-between gap-2 mt-auto pt-1">
              <span class="badge badge-xs badge-ghost font-mono">npm</span>
              <button
                class="btn btn-xs btn-outline btn-primary font-mono"
                @click="importFromBrowse(item.pkg)"
              >→ Import</button>
            </div>
          </div>
        </div>

        <!-- external search CTA -->
        <div class="bg-base-200 border border-base-300/60 rounded-lg p-4 mt-2">
          <p class="text-xs font-mono text-base-content/50 mb-2">Looking for GitHub or Claude Marketplace extensions?</p>
          <button
            class="btn btn-sm btn-ghost font-mono text-xs border border-base-300"
            @click="activeTab = 'import'"
          >Use the Import tab to load from GitHub, local path, or OpenAPI →</button>
        </div>
      </div>

      <!-- ── IMPORT TAB ── -->
      <div v-if="activeTab === 'import'" class="max-w-2xl space-y-5">

        <div class="bg-base-200 border border-base-300/50 rounded-lg px-4 py-3">
          <p class="text-xs font-mono text-base-content/50 leading-relaxed">
            Import from <strong class="text-base-content/70">npm</strong> (MCP servers via npx),
            <strong class="text-base-content/70">GitHub</strong> repos with an <code class="bg-base-300 px-1 rounded">agent007.json</code> manifest,
            <strong class="text-base-content/70">local</strong> directories, or
            <strong class="text-base-content/70">OpenAPI</strong> specs.
            Preview shows exactly what will be installed before you commit.
          </p>
        </div>

        <!-- Source kind -->
        <div class="form-control gap-1.5">
          <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Source type</span></label>
          <select
            v-model="form.source_kind"
            class="select select-sm select-bordered w-full font-mono text-sm"
            @change="bundle = null; previewError = null"
          >
            <option v-for="opt in sourceKindOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
          </select>
        </div>

        <!-- Source ref -->
        <div class="form-control gap-1.5">
          <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60 capitalize">{{ labelFor(form.source_kind) }}</span></label>
          <input
            v-model="form.source_ref"
            type="text"
            class="input input-sm input-bordered w-full font-mono text-sm"
            :placeholder="placeholderFor(form.source_kind)"
          />
        </div>

        <!-- GitHub extra fields -->
        <template v-if="form.source_kind === 'github'">
          <div class="grid grid-cols-2 gap-3">
            <div class="form-control gap-1.5">
              <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Owner</span></label>
              <input v-model="form.github_owner" type="text" placeholder="owner" class="input input-sm input-bordered font-mono text-sm" />
            </div>
            <div class="form-control gap-1.5">
              <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Repo</span></label>
              <input v-model="form.github_repo" type="text" placeholder="repo" class="input input-sm input-bordered font-mono text-sm" />
            </div>
          </div>
          <div class="form-control gap-1.5">
            <label class="label py-0"><span class="label-text text-xs font-mono text-base-content/50">Branch / ref <span class="opacity-60">(optional, defaults to main)</span></span></label>
            <input v-model="form.github_ref" type="text" placeholder="main" class="input input-sm input-bordered font-mono text-sm" />
          </div>
        </template>

        <div class="flex justify-end">
          <button
            class="btn btn-sm btn-primary font-mono px-6"
            :disabled="previewing || !form.source_ref.trim()"
            @click="preview"
          >
            <span v-if="previewing" class="loading loading-spinner loading-xs" />
            {{ previewing ? 'Previewing…' : 'Preview' }}
          </button>
        </div>

        <!-- Preview error -->
        <div v-if="previewError" class="alert alert-error text-xs font-mono py-2.5 px-4">
          <span class="text-base">⚠</span>
          <span>{{ previewError }}</span>
        </div>

        <!-- Bundle preview card -->
        <div v-if="bundle" class="bg-base-200 border border-base-300/60 rounded-lg overflow-hidden">
          <!-- bundle header -->
          <div class="px-5 py-4 border-b border-base-300/50 flex items-start justify-between gap-4">
            <div class="min-w-0">
              <div class="flex items-center gap-2 flex-wrap mb-1">
                <span class="font-mono font-bold text-sm text-base-content/90">
                  {{ bundle.manifest?.extension?.name ?? '(unnamed bundle)' }}
                </span>
                <span v-if="bundle.manifest?.extension?.version" class="font-mono text-xs text-base-content/40">
                  v{{ bundle.manifest.extension.version }}
                </span>
              </div>
              <p v-if="bundle.manifest?.extension?.description" class="text-xs text-base-content/55 font-mono">
                {{ bundle.manifest.extension.description }}
              </p>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <span
                v-if="bundle.compat_grade"
                class="badge font-mono font-bold text-xs"
                :class="gradeClass(bundle.compat_grade)"
                :title="gradeLabel(bundle.compat_grade)"
              >Grade {{ bundle.compat_grade }}</span>
            </div>
          </div>

          <!-- component checklist -->
          <div class="px-5 py-4 space-y-2">
            <p class="text-[10px] font-mono text-base-content/40 uppercase tracking-widest mb-2.5">Components to install</p>
            <label v-if="(bundle.skills?.length ?? 0) > 0" class="flex items-center gap-2.5 text-xs font-mono cursor-pointer">
              <input type="checkbox" v-model="compFlags.skills" class="checkbox checkbox-xs checkbox-primary" />
              <span class="text-base-content/70">{{ bundle.skills.length }} skill{{ bundle.skills.length !== 1 ? 's' : '' }}</span>
              <span class="text-base-content/30 text-[10px]">→ ~/.agent007/skills/</span>
            </label>
            <label v-if="(bundle.tools?.length ?? 0) > 0" class="flex items-center gap-2.5 text-xs font-mono cursor-pointer">
              <input type="checkbox" v-model="compFlags.tools" class="checkbox checkbox-xs checkbox-primary" />
              <span class="text-base-content/70">{{ bundle.tools.length }} tool{{ bundle.tools.length !== 1 ? 's' : '' }}</span>
              <span class="text-base-content/30 text-[10px]">→ ~/.agent007/tools/</span>
            </label>
            <label v-if="(bundle.workflows?.length ?? 0) > 0" class="flex items-center gap-2.5 text-xs font-mono cursor-pointer">
              <input type="checkbox" v-model="compFlags.workflows" class="checkbox checkbox-xs checkbox-primary" />
              <span class="text-base-content/70">{{ bundle.workflows.length }} workflow{{ bundle.workflows.length !== 1 ? 's' : '' }}</span>
              <span class="text-base-content/30 text-[10px]">→ ~/.agent007/workflows/</span>
            </label>
            <label v-if="(bundle.mcp_servers?.length ?? 0) > 0" class="flex items-center gap-2.5 text-xs font-mono cursor-pointer">
              <input type="checkbox" v-model="compFlags.mcp" class="checkbox checkbox-xs checkbox-primary" />
              <span class="text-base-content/70">{{ bundle.mcp_servers.length }} MCP server{{ bundle.mcp_servers.length !== 1 ? 's' : '' }}</span>
              <span class="text-base-content/30 text-[10px]">→ registered in MCP tab</span>
            </label>
            <label v-if="(bundle.rag_sources?.length ?? 0) > 0" class="flex items-center gap-2.5 text-xs font-mono cursor-pointer">
              <input type="checkbox" v-model="compFlags.rag" class="checkbox checkbox-xs checkbox-primary" />
              <span class="text-base-content/70">{{ bundle.rag_sources.length }} RAG source{{ bundle.rag_sources.length !== 1 ? 's' : '' }}</span>
              <span class="text-base-content/30 text-[10px]">→ registered in Memory → RAG</span>
            </label>
            <p
              v-if="!bundle.skills?.length && !bundle.tools?.length && !bundle.workflows?.length && !bundle.mcp_servers?.length && !bundle.rag_sources?.length"
              class="text-xs text-base-content/40 font-mono italic"
            >No components detected in this bundle</p>
          </div>

          <!-- warnings -->
          <div v-if="bundle.warnings?.length" class="px-5 pb-4 space-y-1.5">
            <div v-for="(w, i) in bundle.warnings" :key="i" class="alert alert-warning py-2 px-3 text-xs font-mono">{{ w }}</div>
          </div>

          <!-- install button -->
          <div class="px-5 py-4 border-t border-base-300/50 flex justify-end">
            <button
              class="btn btn-sm btn-success font-mono px-6"
              :disabled="installing"
              @click="install"
            >
              <span v-if="installing" class="loading loading-spinner loading-xs" />
              Install Extension
            </button>
          </div>
        </div>
      </div>

      <!-- ── INSTALLED TAB ── -->
      <div v-if="activeTab === 'installed'">
        <div v-if="loadingInstalled" class="flex items-center justify-center h-24">
          <span class="loading loading-spinner loading-sm text-primary" />
        </div>

        <div v-else-if="!installed.length" class="text-center py-16">
          <div class="text-4xl opacity-10 mb-3">⊞</div>
          <p class="font-mono text-sm text-base-content/40">No extensions installed yet</p>
          <p class="font-mono text-xs text-base-content/30 mt-1">Browse the curated list or use Import to get started</p>
        </div>

        <div v-else class="overflow-x-auto">
          <table class="table table-sm w-full font-mono">
            <thead>
              <tr class="text-base-content/40 text-[10px] uppercase tracking-widest">
                <th>Name</th>
                <th>Grade</th>
                <th>Components</th>
                <th>Installed</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="ext in installed" :key="ext.name" class="hover:bg-base-300/30">
                <td class="font-semibold text-base-content/90">
                  {{ ext.name }}
                  <span v-if="ext.version" class="text-base-content/35 font-normal ml-1">v{{ ext.version }}</span>
                </td>
                <td>
                  <span
                    v-if="ext.compat_grade"
                    class="badge badge-xs font-bold"
                    :class="gradeClass(ext.compat_grade)"
                    :title="gradeLabel(ext.compat_grade)"
                  >{{ ext.compat_grade }}</span>
                  <span v-else class="text-base-content/30">—</span>
                </td>
                <td class="text-base-content/60 text-xs">{{ compSummary(ext) }}</td>
                <td class="text-base-content/40 text-xs">{{ fmtDate(ext.installed_at) }}</td>
                <td>
                  <span class="btn btn-xs btn-ghost font-mono text-base-content/25 cursor-not-allowed" title="Manual removal: delete files from ~/.agent007/ and re-run agent007 init">uninstall</span>
                </td>
              </tr>
            </tbody>
          </table>
          <p class="text-xs font-mono text-base-content/25 mt-3 px-1">
            To uninstall: manually delete files from <code class="bg-base-200 px-1 rounded">~/.agent007/</code> then run <code class="bg-base-200 px-1 rounded">agent007 init</code>.
          </p>
        </div>
      </div>

    </div><!-- /content -->

    <!-- Toast -->
    <Transition name="toast">
      <div
        v-if="toast"
        class="fixed bottom-5 right-5 z-50 alert shadow-lg max-w-sm py-2.5 px-4 font-mono text-sm"
        :class="toast.type === 'error' ? 'alert-error' : 'alert-success'"
      >{{ toast.msg }}</div>
    </Transition>
  </div>
</template>

<style scoped>
.toast-enter-active, .toast-leave-active { transition: opacity 0.25s, transform 0.25s; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(8px); }
</style>

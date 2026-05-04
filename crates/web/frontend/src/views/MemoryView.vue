<script setup>
import { ref, computed, onMounted, watch, nextTick } from 'vue'
import { useApi } from '../composables/useApi.js'
import { marked } from 'marked'
import DOMPurify from 'dompurify'

// Configure marked for GFM
marked.setOptions({ gfm: true, breaks: true })

const { api } = useApi()

const SCOPES = ['global', 'project', 'user', 'learning']
const SCOPE_ICONS = { global: '◎', project: '⬡', user: '◉', learning: '◈' }

const activeScope = ref('project')
const keys = ref([])
const selectedKey = ref(null)
const content = ref('')
const loadingKeys = ref(false)
const loadingContent = ref(false)
const contentError = ref(null)
const searchQuery = ref('')
const copyStatus = ref('')
const contentVisible = ref(false)
const scopeStats = ref(null)

onMounted(() => loadKeys('project'))

const filteredKeys = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return keys.value
  return keys.value.filter(k => k.toLowerCase().includes(q))
})

// Split colon-key into visual path segments
function keySegments(key) {
  return key.split(/[:/]/).filter(Boolean)
}

async function loadKeys(scope) {
  activeScope.value = scope
  selectedKey.value = null
  content.value = ''
  contentError.value = null
  contentVisible.value = false
  searchQuery.value = ''
  loadingKeys.value = true
  scopeStats.value = null
  try {
    const [list, stats] = await Promise.all([
      api.listMemory(scope).catch(() => []),
      api.getMemoryStats(scope).catch(() => null),
    ])
    keys.value = list || []
    scopeStats.value = stats
  } catch {
    keys.value = []
  } finally {
    loadingKeys.value = false
  }
}

async function selectKey(key) {
  if (selectedKey.value === key) return
  selectedKey.value = key
  content.value = ''
  contentError.value = null
  contentVisible.value = false
  loadingContent.value = true
  try {
    content.value = await api.getMemory(activeScope.value, key)
    setTimeout(() => { contentVisible.value = true }, 20)
  } catch (e) {
    contentError.value = e.message
    contentVisible.value = true
  } finally {
    loadingContent.value = false
  }
}

async function copyContent() {
  if (!content.value) return
  try {
    await navigator.clipboard.writeText(content.value)
    copyStatus.value = 'Copied!'
    setTimeout(() => { copyStatus.value = '' }, 2000)
  } catch {
    copyStatus.value = 'Failed'
    setTimeout(() => { copyStatus.value = '' }, 2000)
  }
}

// Render markdown via marked (GFM) + DOMPurify (XSS safe)
function renderMarkdown(raw) {
  if (!raw) return ''
  const dirty = marked.parse(raw)
  return DOMPurify.sanitize(dirty, {
    ADD_TAGS: ['pre', 'code'],
    ADD_ATTR: ['class'],
  })
}

// ── RAG Sources ─────────────────────────────────────────────────────────────
const memoryTab = ref('memory')  // 'memory' | 'rag'

const ragSources = ref([])
const ragLoading = ref(false)
const ragAddForm = ref({ name: '', kind: 'url', source_ref: '', scope: 'project', chunk_size: 512 })
const ragAddBusy = ref(false)
const ragQuery   = ref('')
const ragQueryBusy = ref(false)
const ragQueryResults = ref(null)
const ragBusyMap = ref({})

const ragToast = ref(null)
let ragToastTimer = null
function showRagToast(msg, type = 'success') {
  clearTimeout(ragToastTimer)
  ragToast.value = { msg, type }
  ragToastTimer = setTimeout(() => { ragToast.value = null }, 3500)
}

function ragStatusClass(status) {
  return {
    ready:    'badge-success',
    indexing: 'badge-warning',
    pending:  'badge-ghost',
    error:    'badge-error',
  }[status] || 'badge-ghost'
}

async function loadRagSources() {
  ragLoading.value = true
  try {
    const data = await api.listRagSources()
    ragSources.value = data?.sources ?? []
  } catch (e) {
    showRagToast(e.message, 'error')
  } finally {
    ragLoading.value = false
  }
}

async function addRagSource() {
  if (!ragAddForm.value.name.trim() || !ragAddForm.value.source_ref.trim()) {
    showRagToast('Name and source are required', 'error')
    return
  }
  ragAddBusy.value = true
  try {
    const source = await api.addRagSource({ ...ragAddForm.value })
    ragSources.value.push(source)
    showRagToast(`${source.name} added — indexing in background`)
    ragAddForm.value = { name: '', kind: 'url', source_ref: '', scope: 'project', chunk_size: 512 }
  } catch (e) {
    showRagToast(e.message, 'error')
  } finally {
    ragAddBusy.value = false
  }
}

async function reindexRag(id) {
  ragBusyMap.value[id] = true
  try {
    const updated = await api.reindexRagSource(id)
    const idx = ragSources.value.findIndex(s => s.id === id)
    if (idx !== -1) ragSources.value[idx] = updated
    showRagToast(`${id} reindexed`)
  } catch (e) {
    showRagToast(e.message, 'error')
  } finally {
    delete ragBusyMap.value[id]
  }
}

async function deleteRagSource(id) {
  if (!confirm(`Remove RAG source '${id}'?`)) return
  try {
    await api.deleteRagSource(id)
    ragSources.value = ragSources.value.filter(s => s.id !== id)
    showRagToast(`${id} removed`)
  } catch (e) {
    showRagToast(e.message, 'error')
  }
}

async function testRagQuery() {
  if (!ragQuery.value.trim()) return
  ragQueryBusy.value = true
  try {
    ragQueryResults.value = await api.queryRag(ragQuery.value.trim())
  } catch (e) {
    showRagToast(e.message, 'error')
  } finally {
    ragQueryBusy.value = false
  }
}

// After content renders, find all .language-mermaid blocks and render them as SVG.
// Mermaid is lazy-loaded on first use to avoid bundling it into the main chunk.
let mermaidReady = false
async function ensureMermaid() {
  if (mermaidReady) return
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
    themeVariables: {
      background: '#1d232a',
      primaryColor: '#7480ff',
      primaryTextColor: '#a6adbb',
      primaryBorderColor: '#2a323c',
      lineColor: '#4b5563',
      secondaryColor: '#2a323c',
      tertiaryColor: '#191e24',
      edgeLabelBackground: '#1d232a',
      fontFamily: 'ui-monospace, monospace',
    },
  })
  mermaidReady = true
  return mermaid
}

watch(contentVisible, async (visible) => {
  if (!visible) return
  await nextTick()
  const blocks = document.querySelectorAll('.md-preview .language-mermaid')
  if (!blocks.length) return
  blocks.forEach(block => {
    const source = block.textContent || ''
    const container = block.closest('pre') || block
    container.outerHTML = `<div class="mermaid-block">${source}</div>`
  })
  await nextTick()
  try {
    const mermaid = await ensureMermaid()
    await mermaid.run({ querySelector: '.mermaid-block' })
  } catch (e) {
    console.warn('mermaid render error:', e)
  }
})
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">

    <!-- ── Top bar ─────────────────────────────────────────────────────── -->
    <div class="flex items-center gap-3 px-5 py-3 border-b border-base-300 bg-base-200 flex-shrink-0">
      <span class="text-primary font-mono font-bold tracking-widest text-sm uppercase">◈ Memory</span>
      <div class="h-4 w-px bg-base-300"></div>

      <!-- Main tab switcher -->
      <div class="flex gap-0.5 bg-base-300/60 rounded p-0.5">
        <button
          class="btn btn-xs font-mono rounded px-3"
          :class="memoryTab === 'memory' ? 'btn-primary shadow-sm' : 'btn-ghost text-base-content/50'"
          @click="memoryTab = 'memory'"
        >KV Store</button>
        <button
          class="btn btn-xs font-mono rounded px-3"
          :class="memoryTab === 'rag' ? 'btn-primary shadow-sm' : 'btn-ghost text-base-content/50'"
          @click="memoryTab = 'rag'; loadRagSources()"
        >RAG Sources</button>
      </div>

      <!-- Scope pills — KV tab only -->
      <div v-if="memoryTab === 'memory'" class="flex gap-1">
        <button
          v-for="s in SCOPES" :key="s"
          class="btn btn-xs font-mono gap-1 rounded-full border border-base-300 bg-base-300/40 text-base-content/50 hover:border-primary/50 hover:text-base-content/80"
          :class="activeScope === s ? 'border-primary/70 bg-primary/10 text-primary' : ''"
          @click="loadKeys(s)"
        >
          <span class="text-[0.72rem]">{{ SCOPE_ICONS[s] }}</span>
          <span class="text-[0.68rem] tracking-wide">{{ s }}</span>
        </button>
      </div>

      <div class="flex-1"></div>

      <!-- Count badge -->
      <span v-if="memoryTab === 'memory'" class="badge badge-ghost badge-sm font-mono text-base-content/30">
        {{ loadingKeys ? '…' : `${filteredKeys.length} / ${keys.length}` }}
      </span>
      <span v-else class="badge badge-ghost badge-sm font-mono text-base-content/30">
        {{ ragSources.length }} source{{ ragSources.length !== 1 ? 's' : '' }}
      </span>

      <template v-if="memoryTab === 'memory' && scopeStats">
        <span class="badge badge-ghost badge-sm font-mono text-base-content/35">
          Σ {{ scopeStats.total_keys }}
        </span>
        <span class="badge badge-ghost badge-sm font-mono text-base-content/35">
          P {{ scopeStats.procedural_keys }}
        </span>
        <span class="badge badge-ghost badge-sm font-mono text-base-content/35">
          C {{ Number(scopeStats.avg_confidence || 0).toFixed(2) }}
        </span>
        <span
          v-if="activeScope === 'learning' && scopeStats.learning_skill_count != null"
          class="badge badge-ghost badge-sm font-mono text-base-content/35"
        >
          skills {{ scopeStats.learning_skill_count }}
        </span>
      </template>
    </div>

    <!-- ── RAG Sources panel ────────────────────────────────────────────── -->
    <div v-if="memoryTab === 'rag'" class="flex flex-1 overflow-hidden">
      <div class="flex-1 overflow-y-auto p-6 max-w-4xl">

        <!-- Add source form -->
        <div class="bg-base-200 rounded-lg border border-base-300/60 p-5 mb-6">
          <h3 class="font-mono text-xs font-bold text-base-content/60 uppercase tracking-wider mb-4">Add Knowledge Source</h3>
          <div class="space-y-3">
            <!-- Row 1: Name full-width -->
            <div class="form-control gap-1.5">
              <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Name</span></label>
              <input v-model="ragAddForm.name" type="text" class="input input-bordered input-sm font-mono text-sm w-full"
                placeholder="project-docs" />
            </div>
            <!-- Row 2: Kind + Scope -->
            <div class="grid grid-cols-2 gap-3">
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Kind</span></label>
                <select v-model="ragAddForm.kind" class="select select-bordered select-sm font-mono text-sm w-full">
                  <option value="url">URL</option>
                  <option value="file">File</option>
                  <option value="directory">Directory</option>
                </select>
              </div>
              <div class="form-control gap-1.5">
                <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Scope</span></label>
                <select v-model="ragAddForm.scope" class="select select-bordered select-sm font-mono text-sm w-full">
                  <option value="project">project</option>
                  <option value="global">global</option>
                </select>
              </div>
            </div>
            <!-- Row 3: Source Ref full-width -->
            <div class="form-control gap-1.5">
              <label class="label py-0"><span class="label-text text-xs font-mono font-semibold text-base-content/60">Source</span></label>
              <input v-model="ragAddForm.source_ref" type="text" class="input input-bordered input-sm font-mono text-sm w-full"
                :placeholder="ragAddForm.kind === 'url' ? 'https://docs.example.com/api' : ragAddForm.kind === 'file' ? '/path/to/document.md' : '/path/to/docs-directory'" />
            </div>
            <!-- Row 4: Chunk Size + hint -->
            <div class="form-control gap-1.5">
              <label class="label py-0">
                <span class="label-text text-xs font-mono font-semibold text-base-content/60">Chunk Size</span>
                <span class="label-text-alt text-[10px] font-mono text-base-content/30">tokens per chunk · 128–4096</span>
              </label>
              <input v-model.number="ragAddForm.chunk_size" type="number" class="input input-bordered input-sm font-mono text-sm w-32"
                min="128" max="4096" step="128" />
            </div>
            <!-- Row 5: Submit right-aligned -->
            <div class="flex justify-end pt-1">
              <button class="btn btn-primary btn-sm font-mono px-5"
                :class="ragAddBusy && 'loading'"
                :disabled="ragAddBusy || !ragAddForm.name.trim() || !ragAddForm.source_ref.trim()"
                @click="addRagSource">
                Add Source
              </button>
            </div>
          </div>
        </div>

        <!-- Sources list -->
        <div v-if="ragLoading" class="text-center py-8 font-mono text-xs text-base-content/30">Loading…</div>
        <div v-else-if="!ragSources.length" class="text-center py-12 font-mono text-xs text-base-content/30">
          No RAG sources yet. Add a URL, file, or directory above to index knowledge.
        </div>
        <div v-else class="space-y-2.5 mb-6">
          <div
            v-for="s in ragSources" :key="s.id"
            class="bg-base-200 rounded-lg border border-base-300/60 px-4 py-3.5 flex items-start justify-between gap-4"
          >
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2 flex-wrap mb-1.5">
                <span class="font-mono text-sm font-semibold text-base-content/90">{{ s.name }}</span>
                <span class="badge badge-xs badge-outline font-mono">{{ s.kind }}</span>
                <span class="badge badge-xs font-mono" :class="ragStatusClass(s.status)">{{ s.status }}</span>
                <span class="badge badge-xs badge-ghost font-mono">{{ s.scope }}</span>
              </div>
              <p class="font-mono text-xs text-base-content/50 break-all leading-snug">{{ s.source_ref }}</p>
              <div v-if="s.chunk_count != null || s.indexed_at" class="flex gap-3 mt-1.5">
                <span v-if="s.chunk_count != null" class="font-mono text-xs text-base-content/40">{{ s.chunk_count }} chunks</span>
                <span v-if="s.indexed_at" class="font-mono text-xs text-base-content/30">indexed {{ s.indexed_at.slice(0,10) }}</span>
              </div>
              <p v-if="s.error_msg" class="font-mono text-xs text-error/70 mt-1.5">{{ s.error_msg }}</p>
            </div>
            <div class="flex gap-1.5 shrink-0 pt-0.5">
              <button
                class="btn btn-xs btn-ghost font-mono gap-1"
                :class="ragBusyMap[s.id] && 'loading'"
                :disabled="!!ragBusyMap[s.id]"
                @click="reindexRag(s.id)"
                title="Reindex"
              >↺ reindex</button>
              <button
                class="btn btn-xs btn-ghost text-error font-mono"
                @click="deleteRagSource(s.id)"
                title="Remove"
              >✕</button>
            </div>
          </div>
        </div>

        <!-- Test query -->
        <div class="bg-base-200 rounded-lg border border-base-300/60 p-5">
          <h3 class="font-mono text-xs font-bold text-base-content/60 uppercase tracking-wider mb-3">Test Query</h3>
          <div class="flex gap-2">
            <input
              v-model="ragQuery"
              type="text"
              class="input input-bordered input-sm font-mono text-sm flex-1"
              placeholder="Ask a question or enter a keyword…"
              @keyup.enter="testRagQuery"
            />
            <button
              class="btn btn-sm btn-outline font-mono shrink-0"
              :class="ragQueryBusy && 'loading'"
              :disabled="ragQueryBusy || !ragQuery.trim()"
              @click="testRagQuery"
            >Query</button>
          </div>
          <div v-if="ragQueryResults" class="mt-4 space-y-2">
            <p class="font-mono text-xs text-base-content/40">Searched {{ ragQueryResults.sources_searched }} source(s)</p>
            <div v-if="ragQueryResults.results?.length" class="space-y-1.5">
              <div
                v-for="r in ragQueryResults.results" :key="r.id"
                class="bg-base-300/40 rounded px-3 py-2 font-mono text-xs flex items-center gap-3"
              >
                <span class="text-primary font-semibold">{{ r.name }}</span>
                <span class="badge badge-xs badge-ghost">{{ r.kind }}</span>
                <span class="text-base-content/40">{{ r.status }}</span>
              </div>
            </div>
            <p v-else class="font-mono text-xs text-base-content/30">No ready sources found — sources must have status "ready" to appear in results.</p>
          </div>
        </div>

      </div>
    </div>

    <!-- ── KV Memory Body ──────────────────────────────────────────────── -->
    <div v-if="memoryTab === 'memory'" class="flex flex-1 overflow-hidden">

      <!-- Left: key navigator -->
      <div class="w-60 flex-shrink-0 border-r border-base-300 flex flex-col overflow-hidden bg-base-200/50">

        <!-- Search -->
        <div class="px-3 pt-3 pb-2 flex-shrink-0">
          <label class="input input-sm input-bordered flex items-center gap-2 bg-base-300/60 font-mono text-xs">
            <span class="text-base-content/30 text-sm">⌕</span>
            <input
              v-model="searchQuery"
              class="grow min-w-0"
              placeholder="Filter keys…"
              type="text"
              spellcheck="false"
            />
            <button v-if="searchQuery" class="text-base-content/30 hover:text-base-content/70 cursor-pointer" @click="searchQuery = ''">&times;</button>
          </label>
        </div>

        <!-- Key list -->
        <div class="flex-1 overflow-y-auto px-2 pb-3">

          <!-- Skeleton loader -->
          <template v-if="loadingKeys">
            <div
              v-for="i in 6" :key="i"
              class="skeleton h-8 w-full rounded-md mb-1"
              :style="`opacity:${1 - i * 0.12}`"
            ></div>
          </template>

          <!-- Keys -->
          <template v-else>
            <button
              v-for="key in filteredKeys"
              :key="key"
              class="w-full flex items-center gap-2 px-2 py-1.5 rounded-md border border-transparent bg-transparent text-left cursor-pointer transition-all duration-100 mb-0.5 hover:bg-base-300/70 hover:border-base-300"
              :class="selectedKey === key ? 'bg-primary/10 border-primary/35' : ''"
              @click="selectKey(key)"
              :title="key"
            >
              <!-- Dot indicator -->
              <span
                class="w-1.5 h-1.5 rounded-full flex-shrink-0 transition-all duration-150"
                :class="selectedKey === key ? 'bg-primary shadow-[0_0_6px_oklch(var(--p)/0.5)]' : 'bg-base-content/20'"
              ></span>

              <!-- Key path segments -->
              <span class="font-mono text-[0.72rem] leading-snug overflow-hidden text-ellipsis whitespace-nowrap flex-1">
                <span
                  v-for="(seg, idx) in keySegments(key)"
                  :key="idx"
                >
                  <span v-if="idx > 0" class="text-base-content/20 text-[0.62rem]"> › </span>
                  <span :class="idx === keySegments(key).length - 1
                    ? (selectedKey === key ? 'text-primary font-semibold' : 'text-base-content/85 font-semibold')
                    : (selectedKey === key ? 'text-primary/60' : 'text-base-content/45')"
                  >{{ seg }}</span>
                </span>
              </span>
            </button>

            <!-- Empty state -->
            <div v-if="!filteredKeys.length" class="py-6 px-2 text-center font-mono text-[0.72rem] text-base-content/25">
              <div class="text-xl mb-1.5">{{ searchQuery ? '⊘' : '◈' }}</div>
              <div>{{ searchQuery ? 'No matches' : 'Empty scope' }}</div>
            </div>
          </template>
        </div>
      </div>

      <!-- Right: content viewer -->
      <div class="flex-1 flex flex-col overflow-hidden">

        <!-- Preview toolbar -->
        <div class="flex items-center gap-2 px-4 h-9 bg-base-200/60 border-b border-base-300 flex-shrink-0">
          <template v-if="selectedKey">
            <!-- Breadcrumb -->
            <div class="flex-1 flex items-center gap-0 font-mono text-[0.72rem] overflow-hidden text-ellipsis whitespace-nowrap min-w-0">
              <span class="text-base-content/40 text-[0.7rem] flex-shrink-0">{{ SCOPE_ICONS[activeScope] }} {{ activeScope }}</span>
              <span class="text-base-content/20 mx-1 flex-shrink-0"> / </span>
              <span v-for="(seg, idx) in keySegments(selectedKey)" :key="idx" class="flex items-center flex-shrink-0">
                <span v-if="idx > 0" class="text-base-content/20 mx-0.5"> › </span>
                <span :class="idx === keySegments(selectedKey).length - 1 ? 'text-primary/90 font-semibold' : 'text-base-content/45'">{{ seg }}</span>
              </span>
            </div>
            <button v-if="content && !loadingContent" class="btn btn-xs btn-ghost font-mono flex-shrink-0" @click="copyContent">
              {{ copyStatus || '⎘ Copy' }}
            </button>
          </template>
          <span v-else class="font-mono text-[0.72rem] text-base-content/20 italic">← select a key</span>
          <span v-if="loadingContent" class="loading loading-spinner loading-xs ml-auto text-primary/40"></span>
        </div>

        <!-- Content body -->
        <div class="flex-1 overflow-y-auto p-7 relative">

          <!-- Skeleton content loader -->
          <div v-if="loadingContent" class="space-y-2.5 py-1">
            <div class="skeleton h-5 w-1/2 rounded"></div>
            <div class="skeleton h-3 w-full rounded"></div>
            <div class="skeleton h-3 w-3/4 rounded"></div>
            <div class="skeleton h-3 w-1/2 rounded"></div>
            <div class="skeleton h-4 w-2/5 rounded mt-5"></div>
            <div class="skeleton h-3 w-full rounded"></div>
            <div class="skeleton h-3 w-5/6 rounded"></div>
          </div>

          <!-- Error -->
          <div v-else-if="contentError" class="flex flex-col items-center justify-center py-10 text-error font-mono text-center">
            <div class="text-2xl mb-2">⚠</div>
            <div class="text-xs">{{ contentError }}</div>
          </div>

          <!-- Rendered markdown -->
          <transition name="fade-up">
            <div
              v-if="contentVisible && content && !contentError"
              class="md-preview max-w-3xl font-mono"
              v-html="renderMarkdown(content)"
            ></div>
          </transition>

          <!-- Empty state -->
          <div v-if="!selectedKey && !loadingContent" class="absolute inset-0 flex flex-col items-center justify-center gap-2 pointer-events-none">
            <div class="text-[2.5rem] leading-none text-primary/10">◈</div>
            <div class="font-mono font-bold text-sm text-base-content/15 tracking-widest uppercase">Memory Viewer</div>
            <div class="text-xs text-base-content/20 text-center leading-relaxed">Select a key from the left panel<br>to read its contents</div>
          </div>

        </div>
      </div>

    </div>
  </div>

  <!-- RAG toast -->
  <Transition name="toast">
    <div v-if="ragToast" class="fixed bottom-6 right-6 z-50">
      <div class="alert shadow-lg py-2 px-4 font-mono text-sm max-w-xs"
        :class="{
          'alert-success': ragToast.type === 'success',
          'alert-error':   ragToast.type === 'error',
          'alert-warning': ragToast.type === 'warning',
        }">
        {{ ragToast.msg }}
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* Vue transitions — cannot use utility classes */
.fade-up-enter-active { transition: opacity 0.25s ease, transform 0.25s ease; }
.fade-up-enter-from   { opacity: 0; transform: translateY(6px); }

.toast-enter-active, .toast-leave-active { transition: all .25s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(8px); }

/* ── Markdown preview — :deep() required for v-html injected content ────
   These styles target marked's standard HTML output and cannot use
   Tailwind utility classes since they're not in Vue's template scope.  */
.md-preview :deep(h1),
.md-preview :deep(h2),
.md-preview :deep(h3),
.md-preview :deep(h4) {
  font-family: var(--font-mono);
  font-weight: 700;
  margin: 1.4em 0 0.4em;
  padding-bottom: 0.25em;
  border-bottom: 1px solid oklch(var(--b3));
}
.md-preview :deep(h1) { font-size: 1.5rem;  color: oklch(var(--p)); }
.md-preview :deep(h2) { font-size: 1.2rem;  color: oklch(var(--p) / 0.85); }
.md-preview :deep(h3) { font-size: 1rem;    color: oklch(var(--s)); border-bottom-color: oklch(var(--b3) / 0.5); }
.md-preview :deep(h4) { font-size: 0.88rem; color: oklch(var(--bc) / 0.7); border-bottom: none; font-style: italic; }

.md-preview :deep(p)  { font-size: 0.84rem; line-height: 1.75; color: oklch(var(--bc) / 0.82); margin: 0.5em 0; }
.md-preview :deep(hr) { border: none; border-top: 1px solid oklch(var(--b3)); margin: 1.2em 0; }

.md-preview :deep(code) {
  font-family: var(--font-mono); font-size: 0.79rem;
  background: oklch(var(--b3) / 0.8); color: oklch(var(--s));
  padding: 0.1em 0.4em; border-radius: 4px; border: 1px solid oklch(var(--b3));
}
.md-preview :deep(pre) {
  border-radius: 8px; border: 1px solid oklch(var(--b3));
  overflow: hidden; margin: 0.9em 0; background: oklch(var(--b3) / 0.4);
}
.md-preview :deep(pre code) {
  display: block; background: none; border: none;
  font-size: 0.79rem; line-height: 1.6; color: oklch(var(--bc) / 0.85);
  padding: 14px 16px; overflow-x: auto;
}

.md-preview :deep(ul)   { margin: 0.4em 0 0.4em 1.4em; list-style: disc; }
.md-preview :deep(ol)   { margin: 0.4em 0 0.4em 1.4em; list-style: decimal; }
.md-preview :deep(li)   { font-size: 0.84rem; line-height: 1.65; color: oklch(var(--bc) / 0.8); }

.md-preview :deep(blockquote) {
  border-left: 3px solid oklch(var(--p) / 0.4);
  margin: 0.8em 0; padding: 4px 14px;
  color: oklch(var(--bc) / 0.55); font-style: italic;
}

.md-preview :deep(a)       { color: oklch(var(--p)); text-decoration: underline; text-underline-offset: 2px; }
.md-preview :deep(a:hover) { color: oklch(var(--s)); }

.md-preview :deep(table)  { width: 100%; border-collapse: collapse; font-size: 0.79rem; font-family: var(--font-mono); margin: 0.8em 0; border-radius: 6px; overflow: hidden; border: 1px solid oklch(var(--b3)); }
.md-preview :deep(th)     { background: oklch(var(--b3) / 0.7); color: oklch(var(--bc) / 0.55); font-size: 0.68rem; text-transform: uppercase; letter-spacing: 0.06em; padding: 6px 12px; text-align: left; border-bottom: 1px solid oklch(var(--b3)); }
.md-preview :deep(td)     { padding: 6px 12px; border-bottom: 1px solid oklch(var(--b3) / 0.5); color: oklch(var(--bc) / 0.8); }
.md-preview :deep(tr:last-child td) { border-bottom: none; }
.md-preview :deep(tr:hover td)      { background: oklch(var(--b3) / 0.3); }

.md-preview :deep(strong) { color: oklch(var(--bc)); font-weight: 700; }
.md-preview :deep(em)     { color: oklch(var(--bc) / 0.75); font-style: italic; }
.md-preview :deep(del)    { color: oklch(var(--bc) / 0.35); text-decoration: line-through; }

.md-preview :deep(.mermaid-block) {
  margin: 1em 0; padding: 16px;
  border-radius: 8px; border: 1px solid oklch(var(--b3));
  background: oklch(var(--b3) / 0.25);
  display: flex; justify-content: center; overflow-x: auto;
}
.md-preview :deep(.mermaid-block svg) { max-width: 100%; height: auto; }
</style>

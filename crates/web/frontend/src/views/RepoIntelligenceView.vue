<script setup>
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { useApi } from '../composables/useApi.js'

const props = defineProps({ connected: Boolean, stats: Object })

const { api } = useApi()

const metrics = ref(props.stats || null)
const repoInstallStatus = ref('')
const repoInstallingId = ref('')
const graphActionBusy = ref(false)
const graphActionStatus = ref('')
const graphExplorerMode = ref('impact')
const graphExplorerSymbol = ref('')
const graphExplorerFrom = ref('')
const graphExplorerTo = ref('')
const graphExplorerQuery = ref('')
const graphExplorerDepth = ref(2)
const graphExplorerPaths = ref('')
const graphExplorerBusy = ref(false)
const graphExplorerStatus = ref('')
const graphExplorerResult = ref(null)
const graphExplorerMermaidRef = ref(null)

function mergeStatsSnapshot(previous, incoming) {
  if (!incoming) return previous
  if (!previous) return incoming
  return {
    ...previous,
    ...incoming,
    repo_graph: incoming.repo_graph ?? previous.repo_graph ?? null,
    repo_intelligence: incoming.repo_intelligence ?? previous.repo_intelligence ?? null,
  }
}

watch(() => props.stats, (value) => {
  if (value) metrics.value = mergeStatsSnapshot(metrics.value, value)
}, { deep: true })

const m = computed(() => metrics.value || {
  repo_graph: null,
  repo_intelligence: null,
  runtime_mode: 'hosted-mcp',
})

const repoIntel = computed(() => m.value.repo_intelligence || null)
const repoGraph = computed(() => m.value.repo_graph || null)
const installRecommendations = computed(() => repoIntel.value?.recommendations || [])
const capabilityNotes = computed(() => {
  const notes = []
  if (repoIntel.value?.tree_sitter?.note) {
    notes.push({
      id: 'tree-sitter',
      label: 'tree-sitter',
      body: repoIntel.value.tree_sitter.note,
    })
  }
  return notes
})

const graphMissing = computed(() => !repoGraph.value?.exists)
const graphStale = computed(() => !!repoGraph.value?.stale)
const repoIsEmpty = computed(() => repoIntel.value?.state === 'empty_repo')

function repoGraphState(graph) {
  if (!graph?.exists) return 'missing'
  if (graph?.stale) return 'stale'
  return 'ready'
}

function repoGraphBadgeClass(graph) {
  const state = repoGraphState(graph)
  return {
    'badge-error': state === 'missing',
    'badge-warning': state === 'stale',
    'badge-success': state === 'ready',
  }
}

function repoIntelligenceBadgeClass(readiness) {
  const state = readiness?.state || 'baseline_only'
  if (state === 'enrichment_active') return 'badge-success'
  if (state === 'enrichment_available') return 'badge-warning'
  if (state === 'empty_repo') return 'badge-ghost'
  return 'badge-neutral'
}

function splitGraphPaths(raw) {
  return (raw || '')
    .split(/\r?\n|,/)
    .map(v => v.trim())
    .filter(Boolean)
}

async function refreshRepoIntelligence() {
  const stats = await api.getStats()
  if (stats) metrics.value = stats
}

async function copyText(value) {
  if (!value) return
  await navigator.clipboard.writeText(value)
}

async function runRepoRecommendation(rec) {
  if (!rec?.id) return
  repoInstallingId.value = rec.id
  repoInstallStatus.value = ''
  try {
    const result = await api.installRepoIntelligence(rec.id, true)
    if (result?.ok) {
      repoInstallStatus.value = `${rec.title} finished`
    } else {
      repoInstallStatus.value = result?.stderr || result?.error || `${rec.title} failed`
    }
    await refreshRepoIntelligence()
  } catch (error) {
    repoInstallStatus.value = error?.message || String(error)
  } finally {
    repoInstallingId.value = ''
  }
}

async function runGraphAction(tool) {
  graphActionBusy.value = true
  graphActionStatus.value = ''
  try {
    const input = tool === 'etr.graph_refresh_paths'
      ? { paths: splitGraphPaths(graphExplorerPaths.value) }
      : {}
    await api.etrCall(tool, input, false)
    graphActionStatus.value = `${tool} ok`
    await refreshRepoIntelligence()
  } catch (error) {
    graphActionStatus.value = error?.message || String(error)
  } finally {
    graphActionBusy.value = false
  }
}

let mermaidInstance = null
async function ensureMermaid() {
  if (mermaidInstance) return mermaidInstance
  const mermaid = (await import('mermaid')).default
  mermaid.initialize({
    startOnLoad: false,
    theme: 'dark',
    securityLevel: 'strict',
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
  mermaidInstance = mermaid
  return mermaid
}

const graphExplorerMermaidSource = computed(() => {
  const payload = graphExplorerResult.value?.output
  const tool = graphExplorerResult.value?.tool
  if (!payload || !tool) return ''

  const esc = (value) => String(value || '').replace(/"/g, '\\"')

  if (tool === 'etr.usage_graph' || tool === 'etr.impact_radius') {
    const nodes = payload.nodes || []
    const edges = payload.edges || []
    if (!nodes.length) return ''
    const ids = new Map(nodes.map((node, index) => [node.id, `N${index + 1}`]))
    const lines = ['graph TD']
    for (const node of nodes) {
      const nodeId = ids.get(node.id)
      const label = `${node.name}${node.path ? `\\n${node.path}` : ''}`
      lines.push(`  ${nodeId}["${esc(label)}"]`)
    }
    for (const edge of edges) {
      const from = ids.get(edge.from)
      const to = ids.get(edge.to)
      if (!from || !to) continue
      lines.push(`  ${from} -->|${esc(edge.kind)}| ${to}`)
    }
    return lines.join('\n')
  }

  if (tool === 'etr.callers') {
    const rows = payload.callers || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  C${index}["${esc(row.caller)}"] --> T${index}["${esc(row.callee)}"]`
    )].join('\n')
  }

  if (tool === 'etr.callees') {
    const rows = payload.callees || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  C${index}["${esc(row.caller)}"] --> T${index}["${esc(row.callee)}"]`
    )].join('\n')
  }

  if (tool === 'etr.doc_links') {
    const rows = payload.docs || []
    if (!rows.length) return ''
    return ['graph TD', ...rows.map((row, index) =>
      `  D${index}["${esc(row.doc)}"] -.-> S${index}["${esc(row.symbol)}"]`
    )].join('\n')
  }

  if (tool === 'etr.dep_path') {
    const steps = payload.steps || []
    if (!steps.length) return ''
    const lines = ['graph TD']
    steps.forEach((step, index) => {
      lines.push(`  P${index}["${esc(step.from_name)}"] -->|${esc(step.edge_kind)}| P${index + 1}["${esc(step.to_name)}"]`)
    })
    return lines.join('\n')
  }

  if (tool === 'etr.context_bundle') {
    const symbols = payload.matched_symbols || []
    const docs = payload.related_docs || []
    if (!symbols.length && !docs.length) return ''
    const lines = ['graph TD']
    symbols.forEach((node, index) => {
      lines.push(`  S${index}["${esc(node.name)}"]`)
    })
    docs.forEach((doc, index) => {
      lines.push(`  D${index}["${esc(doc.name)}"]`)
      if (symbols.length) lines.push(`  D${index} -.-> S0`)
    })
    return lines.join('\n')
  }

  return ''
})

watch(graphExplorerMermaidSource, async () => {
  await renderGraphExplorerMermaid()
})

async function renderGraphExplorerMermaid() {
  await nextTick()
  const root = graphExplorerMermaidRef.value
  const source = graphExplorerMermaidSource.value
  if (!root) return
  root.innerHTML = ''
  if (!source) return
  const target = document.createElement('div')
  target.className = 'artifact-mermaid-block'
  target.textContent = source
  root.appendChild(target)
  try {
    const mermaid = await ensureMermaid()
    await mermaid.run({ nodes: [target] })
  } catch (error) {
    console.warn('repo intelligence mermaid render error:', error)
  }
}

const graphResultSummary = computed(() => {
  const result = graphExplorerResult.value
  if (!result?.output) return []
  const payload = result.output
  const tool = result.tool
  if (tool === 'etr.impact_radius' || tool === 'etr.usage_graph') {
    return [
      `${(payload.nodes || []).length} nodes`,
      `${(payload.edges || []).length} edges`,
      `${(payload.matched_symbols || []).length} matched symbols`,
    ]
  }
  if (tool === 'etr.callers') return [`${(payload.callers || []).length} callers`]
  if (tool === 'etr.callees') return [`${(payload.callees || []).length} callees`]
  if (tool === 'etr.doc_links') return [`${(payload.docs || []).length} linked docs`]
  if (tool === 'etr.dep_path') return [`${(payload.steps || []).length} path steps`]
  if (tool === 'etr.context_bundle') {
    return [
      `${(payload.matched_symbols || []).length} symbols`,
      `${(payload.related_docs || []).length} docs`,
      `${(payload.related_files || []).length} files`,
    ]
  }
  return []
})

async function runGraphExplorer() {
  graphExplorerBusy.value = true
  graphExplorerStatus.value = ''
  try {
    let tool
    let input = {}
    switch (graphExplorerMode.value) {
      case 'usage':
        tool = 'etr.usage_graph'
        input = { symbol: graphExplorerSymbol.value, exact: true, max_depth: Number(graphExplorerDepth.value || 1) }
        break
      case 'impact':
        tool = 'etr.impact_radius'
        input = { symbol: graphExplorerSymbol.value, exact: true, max_depth: Number(graphExplorerDepth.value || 2) }
        break
      case 'callers':
        tool = 'etr.callers'
        input = { symbol: graphExplorerSymbol.value, exact: true }
        break
      case 'callees':
        tool = 'etr.callees'
        input = { symbol: graphExplorerSymbol.value, exact: true }
        break
      case 'docs':
        tool = 'etr.doc_links'
        input = { symbol: graphExplorerSymbol.value, exact: true }
        break
      case 'path':
        tool = 'etr.dep_path'
        input = { from_symbol: graphExplorerFrom.value, to_symbol: graphExplorerTo.value, exact: true }
        break
      case 'context':
        tool = 'etr.context_bundle'
        input = {
          query: graphExplorerQuery.value,
          max_symbols: 6,
          max_neighbors: Math.max(1, Number(graphExplorerDepth.value || 2)),
        }
        break
      default:
        throw new Error('unknown graph explorer mode')
    }

    const output = await api.etrCall(tool, input, false)
    graphExplorerResult.value = { tool, input, output }
    graphExplorerStatus.value = `${tool} ok`
  } catch (error) {
    graphExplorerStatus.value = error?.message || String(error)
  } finally {
    graphExplorerBusy.value = false
  }
}

onMounted(async () => {
  await refreshRepoIntelligence()
})
</script>

<template>
  <div class="flex flex-col h-full overflow-hidden">
    <div class="px-5 py-3 border-b border-base-300 bg-base-200 flex items-center justify-between shrink-0">
      <div class="flex items-center gap-3 min-w-0">
        <span class="text-[11px] font-mono font-bold uppercase tracking-widest text-base-content/40 shrink-0">Repo Intelligence</span>
        <span class="badge badge-xs font-mono" :class="repoGraphBadgeClass(repoGraph)">{{ repoGraphState(repoGraph) }}</span>
        <span v-if="repoIntel" class="badge badge-xs font-mono" :class="repoIntelligenceBadgeClass(repoIntel)">{{ repoIntel.state }}</span>
        <span class="text-[11px] font-mono text-base-content/35 truncate hidden sm:block">
          structural graph + readiness + graph queries
        </span>
      </div>
      <div class="flex items-center gap-2 shrink-0">
        <button class="btn btn-xs btn-ghost font-mono" :disabled="graphActionBusy" @click="refreshRepoIntelligence">refresh stats</button>
        <button class="btn btn-xs btn-primary font-mono" :disabled="graphActionBusy || repoIsEmpty" @click="runGraphAction(graphMissing ? 'etr.graph_build' : 'etr.graph_refresh')">
          {{ graphMissing ? 'build graph' : 'refresh graph' }}
        </button>
      </div>
    </div>

    <div class="flex-1 overflow-auto p-4 space-y-4">
      <div class="grid grid-cols-1 xl:grid-cols-4 gap-3">
        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4">
          <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Graph Health</div>
          <div class="flex items-center gap-2 mb-2">
            <span class="badge badge-sm font-mono" :class="repoGraphBadgeClass(repoGraph)">{{ repoGraphState(repoGraph) }}</span>
            <span class="text-[11px] font-mono text-base-content/40">v{{ repoGraph?.version || '—' }}</span>
          </div>
          <div class="text-[11px] font-mono text-base-content/55">{{ repoGraph?.counts?.symbols || 0 }} symbols · {{ repoGraph?.counts?.edges || 0 }} edges</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1">stale {{ repoGraph?.stale_files || 0 }} · missing {{ repoGraph?.missing_files || 0 }}</div>
        </div>
        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4">
          <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Readiness</div>
          <div class="flex items-center gap-2 mb-2">
            <span class="badge badge-sm font-mono" :class="repoIntelligenceBadgeClass(repoIntel)">{{ repoIntel?.state || 'baseline_only' }}</span>
          </div>
          <div class="text-[11px] font-mono text-base-content/55">{{ repoIntel?.languages?.length || 0 }} detected language{{ (repoIntel?.languages?.length || 0) === 1 ? '' : 's' }}</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1">baseline {{ repoIntel?.baseline_ready ? 'ready' : 'unknown' }}</div>
        </div>
        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4">
          <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Semantic Enrichment</div>
          <div class="text-[11px] font-mono text-base-content/55">tree-sitter {{ repoIntel?.tree_sitter?.status || 'unknown' }}</div>
          <div class="text-[11px] font-mono text-base-content/35 mt-1">
            {{ installRecommendations.length }} install action{{ installRecommendations.length === 1 ? '' : 's' }}
          </div>
        </div>
        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4">
          <div class="text-[9px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Lifecycle</div>
          <div class="text-[11px] font-mono text-base-content/55">1) build when missing</div>
          <div class="text-[11px] font-mono text-base-content/55">2) re-detect when files appear</div>
          <div class="text-[11px] font-mono text-base-content/55">3) ensure before analysis tasks</div>
        </div>
      </div>

      <div v-if="repoIsEmpty" class="rounded-xl border border-base-content/10 bg-base-200 p-4">
        <div class="text-sm font-semibold text-base-content/70 mb-2">Empty project detected</div>
        <div class="text-[12px] font-mono text-base-content/45 leading-relaxed">
          agent007 is ready, but there is no code corpus yet. As soon as manifests or source files appear, readiness will update automatically and this page will offer graph build / enrichment actions.
        </div>
      </div>

      <div v-else class="grid grid-cols-1 xl:grid-cols-[minmax(0,1.1fr)_minmax(22rem,0.9fr)] gap-4">
        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4 space-y-3">
          <div class="flex items-center justify-between gap-3">
            <div>
              <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Detected Languages</div>
              <div class="text-[12px] font-mono text-base-content/45 mt-1">Readiness re-detects when code appears; graph build stays explicit/lazy.</div>
            </div>
            <button
              v-if="graphStale"
              class="btn btn-xs btn-warning font-mono"
              :disabled="graphActionBusy"
              @click="runGraphAction('etr.graph_refresh')"
            >refresh stale graph</button>
          </div>
          <div v-for="lang in (repoIntel?.languages || [])" :key="lang.language" class="rounded-lg border border-base-content/8 px-3 py-3">
            <div class="flex flex-wrap items-center gap-2">
              <span class="text-[12px] font-mono font-bold text-base-content/75">{{ lang.language }}</span>
              <span class="text-[10px] font-mono text-base-content/40">{{ lang.file_count || 0 }} files</span>
              <span class="text-[10px] font-mono text-base-content/40">{{ lang.manifest_count || 0 }} manifests</span>
              <span v-if="lang.lsp" class="badge badge-xs font-mono" :class="lang.lsp.active ? 'badge-success' : (lang.lsp.installed ? 'badge-warning' : 'badge-neutral')">
                lsp {{ lang.lsp.active ? 'active' : (lang.lsp.installed ? 'available' : 'missing') }}
              </span>
            </div>
            <div class="mt-2 text-[10px] font-mono text-base-content/40 break-words">signals: {{ (lang.signals || []).join(', ') || 'none' }}</div>
            <div class="mt-1 text-[10px] font-mono text-base-content/35 break-words">samples: {{ (lang.sample_paths || []).join(', ') || 'none yet' }}</div>
            <div v-if="lang.lsp" class="mt-2 text-[10px] font-mono text-base-content/35 break-words">server: {{ lang.lsp.command }}</div>
          </div>
        </div>

        <div class="rounded-xl border border-base-content/10 bg-base-200 p-4 space-y-3">
          <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Install Actions</div>

          <div v-if="installRecommendations.length" class="space-y-3">
            <div v-for="rec in installRecommendations" :key="rec.id" class="rounded-lg border border-base-content/8 px-3 py-3 space-y-2">
              <div class="flex items-center gap-2">
                <span class="text-[11px] font-mono font-bold text-base-content/75">{{ rec.title }}</span>
                <span class="badge badge-xs font-mono">{{ rec.language }}</span>
              </div>
              <div class="text-[10px] font-mono text-base-content/35 break-all">{{ rec.command }}</div>
              <div class="flex flex-wrap items-center gap-2">
                <button class="btn btn-xs btn-ghost font-mono" @click="copyText(rec.command)">copy</button>
                <button class="btn btn-xs btn-primary font-mono" :disabled="!rec.can_run || repoInstallingId === rec.id" @click="runRepoRecommendation(rec)">
                  {{ repoInstallingId === rec.id ? 'installing…' : 'install' }}
                </button>
                <span v-if="!rec.can_run" class="text-[10px] font-mono text-warning">manual install required</span>
              </div>
            </div>
          </div>

          <div v-else class="rounded-lg border border-base-content/8 px-3 py-3">
            <div class="text-[11px] font-mono text-base-content/55">No remaining install actions.</div>
          </div>

          <div v-if="repoInstallStatus" class="rounded-lg border border-success/20 bg-success/5 px-3 py-3 space-y-1">
            <div class="text-[10px] font-mono uppercase tracking-widest text-success/70">Recent Install Result</div>
            <div class="text-[10px] font-mono text-base-content/60 whitespace-pre-wrap break-words">{{ repoInstallStatus }}</div>
          </div>

          <div v-if="capabilityNotes.length" class="rounded-lg border border-base-content/8 px-3 py-3 space-y-2">
            <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/35">Capability Notes</div>
            <div v-for="note in capabilityNotes" :key="note.id" class="space-y-1">
              <div class="text-[10px] font-mono text-base-content/55">{{ note.label }}</div>
              <div class="text-[10px] font-mono text-base-content/40 leading-relaxed">{{ note.body }}</div>
            </div>
          </div>
        </div>
      </div>

      <div v-if="graphActionStatus" class="rounded-xl border border-base-content/10 bg-base-200 px-4 py-3 text-[11px] font-mono text-base-content/55">
        {{ graphActionStatus }}
      </div>

      <div class="rounded-xl border border-base-content/10 bg-base-200 p-4 space-y-4">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Graph Explorer</div>
            <div class="text-[12px] font-mono text-base-content/40 mt-1">Use this page when you want to interrogate the graph — not from the main dashboard.</div>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <button class="btn btn-xs btn-ghost font-mono" :disabled="graphActionBusy" @click="runGraphAction('etr.graph_build')">build</button>
            <button class="btn btn-xs btn-ghost font-mono" :disabled="graphActionBusy" @click="runGraphAction('etr.graph_refresh')">refresh</button>
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <select v-model="graphExplorerMode" class="select select-sm select-bordered font-mono">
            <option value="impact">impact radius</option>
            <option value="usage">usage neighborhood</option>
            <option value="callers">callers</option>
            <option value="callees">callees</option>
            <option value="docs">doc links</option>
            <option value="path">dependency path</option>
            <option value="context">context bundle</option>
          </select>
          <input v-if="['usage','impact','callers','callees','docs'].includes(graphExplorerMode)" v-model="graphExplorerSymbol" class="input input-sm input-bordered font-mono w-64" placeholder="symbol" />
          <template v-if="graphExplorerMode === 'path'">
            <input v-model="graphExplorerFrom" class="input input-sm input-bordered font-mono w-56" placeholder="from symbol" />
            <input v-model="graphExplorerTo" class="input input-sm input-bordered font-mono w-56" placeholder="to symbol" />
          </template>
          <input v-if="graphExplorerMode === 'context'" v-model="graphExplorerQuery" class="input input-sm input-bordered font-mono min-w-[20rem] flex-1" placeholder="query" />
          <input v-if="graphExplorerMode === 'usage' || graphExplorerMode === 'impact' || graphExplorerMode === 'context'" v-model="graphExplorerDepth" type="number" min="1" max="6" class="input input-sm input-bordered font-mono w-24" />
          <button class="btn btn-sm btn-primary font-mono" :disabled="graphExplorerBusy" @click="runGraphExplorer">{{ graphExplorerBusy ? 'running…' : 'run' }}</button>
        </div>

        <details class="rounded-lg border border-base-content/8 bg-base-300/40">
          <summary class="cursor-pointer px-3 py-2 text-[11px] font-mono text-base-content/55">advanced incremental refresh</summary>
          <div class="p-3 pt-0 space-y-2">
            <textarea v-model="graphExplorerPaths" class="textarea textarea-bordered w-full font-mono text-xs min-h-24" placeholder="paths for incremental refresh, one per line or comma-separated"></textarea>
            <button class="btn btn-xs btn-ghost font-mono" :disabled="graphActionBusy" @click="runGraphAction('etr.graph_refresh_paths')">refresh these paths</button>
          </div>
        </details>

        <div v-if="graphExplorerStatus" class="text-[11px] font-mono text-base-content/45">{{ graphExplorerStatus }}</div>

        <div class="grid grid-cols-1 xl:grid-cols-[minmax(0,1.3fr)_minmax(18rem,0.7fr)] gap-4 items-start">
          <div class="rounded-xl border border-base-content/8 bg-base-300/30 min-h-[22rem] p-3">
            <div ref="graphExplorerMermaidRef" class="min-h-[20rem]"></div>
            <div v-if="!graphExplorerMermaidSource" class="text-[11px] font-mono text-base-content/30">no diagram yet</div>
          </div>
          <div class="rounded-xl border border-base-content/8 bg-base-300/30 p-3 space-y-3">
            <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30">Result Summary</div>
            <div v-if="graphExplorerResult" class="space-y-2">
              <div class="text-[11px] font-mono text-base-content/55">{{ graphExplorerResult.tool }}</div>
              <ul class="space-y-1">
                <li v-for="line in graphResultSummary" :key="line" class="text-[11px] font-mono text-base-content/45">{{ line }}</li>
              </ul>
            </div>
            <div v-else class="text-[11px] font-mono text-base-content/30">Run a graph query to inspect structure, callers, impact, or related docs.</div>
          </div>
        </div>

        <div class="rounded-xl border border-base-content/8 bg-base-300/20 p-3">
          <div class="text-[10px] font-mono uppercase tracking-widest text-base-content/30 mb-2">Recommended triggers</div>
          <div class="space-y-1 text-[11px] font-mono text-base-content/45">
            <div>• first-time build from this page when the graph is missing</div>
            <div>• readiness re-detects automatically after files/manifests appear</div>
            <div>• analysis tasks should ensure build/refresh before running</div>
            <div>• dashboard slash commands: <code>/repo-intelligence</code>, <code>/repo-graph-build</code>, <code>/repo-graph-refresh</code></div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

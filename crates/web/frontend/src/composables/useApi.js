import { ref } from 'vue'

const BASE = ''

async function fetchJson(url, opts = {}) {
  const res = await fetch(BASE + url, {
    headers: { 'Content-Type': 'application/json' },
    ...opts,
  })
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`
    try {
      const body = await res.json()
      if (body?.error) message = body.error
    } catch {}
    throw new Error(message)
  }
  return res.json()
}

export function useApi() {
  const loading = ref(false)
  const error = ref(null)

  async function call(fn) {
    loading.value = true
    error.value = null
    try {
      return await fn()
    } catch (e) {
      error.value = e.message
      return null
    } finally {
      loading.value = false
    }
  }

  const api = {
    // Health
    health: () => fetchJson('/api/health'),

    // Personas
    listPersonas: () => fetchJson('/api/personas'),
    savePersona: (data) => fetchJson('/api/personas', { method: 'POST', body: JSON.stringify(data) }),
    deletePersona: (name) => fetchJson(`/api/personas/${encodeURIComponent(name)}`, { method: 'DELETE' }),

    // Skills
    listSkills: () => fetchJson('/api/skills'),
    saveSkill: (data) => fetchJson('/api/skills', { method: 'POST', body: JSON.stringify(data) }),
    getSkill: (trigger, path = null) => {
      const qs = new URLSearchParams()
      if (path) qs.set('path', path)
      const suffix = qs.toString() ? `?${qs.toString()}` : ''
      return fetchJson(`/api/skills/detail/${encodeURIComponent(trigger)}${suffix}`)
    },
    importSkill: (url, opts = {}) =>
      fetchJson('/api/skills/import', {
        method: 'POST',
        body: JSON.stringify({
          url,
          conflict_action: opts.conflict_action,
          alias_trigger: opts.alias_trigger,
        }),
      }),
    previewSkillImport: (url) =>
      fetchJson('/api/skills/preview', { method: 'POST', body: JSON.stringify({ url }) }),
    discoverSkills: (q, sources = [], limit = 12) =>
      fetchJson('/api/skills/discover', {
        method: 'POST',
        body: JSON.stringify({ q, sources, limit }),
      }),
    deleteSkill: (trigger) => fetchJson(`/api/skills/${encodeURIComponent(trigger.replace(/^\//, ''))}`, { method: 'DELETE' }),
    getRegistry: () => fetchJson('/api/skill-registry'),

    // Workflows
    listWorkflows: () => fetchJson('/api/workflows'),
    getWorkflow: (name) => fetchJson(`/api/workflows/${encodeURIComponent(name)}`),
    saveWorkflow: (data) => fetchJson('/api/workflows', { method: 'POST', body: JSON.stringify(data) }),
    deleteWorkflow: (name) => fetchJson(`/api/workflows/${encodeURIComponent(name)}`, { method: 'DELETE' }),

    // Workflow validation
    validateWorkflow: (data) => fetchJson('/api/workflows/validate', { method: 'POST', body: JSON.stringify(data) }),

    // Workflow Templates
    listTemplates: () => fetchJson('/api/workflow-templates'),
    getTemplate: (name) => fetchJson(`/api/workflow-templates/${encodeURIComponent(name)}`),

    // Memory
    listMemory: (scope) => fetchJson(`/api/memory/${encodeURIComponent(scope)}`),
    getMemoryStats: (scope) => fetchJson(`/api/memory/${encodeURIComponent(scope)}/stats`),
    getMemory: (scope, key) => fetch(`/api/memory/${encodeURIComponent(scope)}/${encodeURIComponent(key)}`).then(r => {
      if (!r.ok) throw new Error(`${r.status} ${r.statusText}`)
      return r.text()
    }),
    deleteMemory: (scope, key) =>
      fetchJson(`/api/memory/${encodeURIComponent(scope)}/${encodeURIComponent(key)}`, { method: 'DELETE' }),
    purgeExpiredMemory: (scope) =>
      fetchJson(`/api/memory/${encodeURIComponent(scope)}/purge-expired`, { method: 'POST' }),

    // Status
    getStatus: () => fetchJson('/api/status'),
    getStats: () => fetchJson('/api/stats'),
    getRuntimeSessions: (limit = 12) => fetchJson(`/api/runtime/sessions?limit=${encodeURIComponent(limit)}`),
    getRuntimeMessages: (id) => fetchJson(`/api/runtime/sessions/${encodeURIComponent(id)}/messages`),
    postRuntimeMessage: (id, data) =>
      fetchJson(`/api/runtime/sessions/${encodeURIComponent(id)}/messages`, { method: 'POST', body: JSON.stringify(data) }),
    getProviderStatus: () => fetchJson('/api/providers/status'),
    getScorecards: (limit = 100) => fetchJson(`/api/scorecards?limit=${encodeURIComponent(limit)}`),
    evaluateRegression: (params = {}) => {
      const query = new URLSearchParams()
      Object.entries(params).forEach(([key, value]) => {
        if (value !== undefined && value !== null && value !== '') {
          query.set(key, String(value))
        }
      })
      const suffix = query.toString() ? `?${query.toString()}` : ''
      return fetchJson(`/api/regression/evaluate${suffix}`)
    },
    listRuns: () => fetchJson('/api/runs'),
    cleanupAwaitingRuns: (data) =>
      fetchJson('/api/runs/cleanup-awaiting', { method: 'POST', body: JSON.stringify(data || {}) }),
    getRunDetail: (id) => fetchJson(`/api/runs/${encodeURIComponent(id)}`),
    previewRunArtifact: (id, path) => {
      const qs = new URLSearchParams()
      qs.set('path', path)
      return fetchJson(`/api/runs/${encodeURIComponent(id)}/artifacts/preview?${qs.toString()}`)
    },
    approveRunStep: (id, data) => fetchJson(`/api/runs/${encodeURIComponent(id)}/approval`, { method: 'POST', body: JSON.stringify(data) }),
    resumeRun: (id) => fetchJson(`/api/runs/${encodeURIComponent(id)}/resume`, { method: 'POST' }),

    // Run task
    runTask: (task) => fetchJson('/api/run', { method: 'POST', body: JSON.stringify({ task }) }),

    // Sharing — promote
    promoteSkill: async (trigger) => {
      const res = await fetch(BASE + `/api/skills/${encodeURIComponent(trigger)}/promote`, { method: 'POST' })
      const body = await res.json().catch(() => ({}))
      return { ok: res.ok, status: res.status, body }
    },
    promoteWorkflow: async (name) => {
      const res = await fetch(BASE + `/api/workflows/${encodeURIComponent(name)}/promote`, { method: 'POST' })
      const body = await res.json().catch(() => ({}))
      return { ok: res.ok, status: res.status, body }
    },

    // Sharing — bundle export (returns raw Response so caller can stream blob)
    exportBundle: (skills, workflows, personas, tools) => {
      const params = new URLSearchParams()
      params.set('skills', Array.isArray(skills) ? skills.join(',') : '')
      params.set('workflows', Array.isArray(workflows) ? workflows.join(',') : '')
      params.set('personas', Array.isArray(personas) ? personas.join(',') : '')
      // Only send tools param when the caller has an explicit selection.
      // Omitting it entirely tells the backend to auto-include all tools on disk
      // (backward-compatible "include all" path).  Sending an empty string would
      // be interpreted as __none__ (include nothing), which is wrong for the case
      // where the user simply hasn't interacted with the tools picker.
      if (Array.isArray(tools) && tools.length > 0) {
        params.set('tools', tools.join(','))
      }
      return fetch(BASE + `/api/bundle/export?${params}`)
    },

    // Sharing — bundle import
    importBundle: (bundle, overwrite) =>
      fetchJson('/api/bundle/import', { method: 'POST', body: JSON.stringify({ bundle, overwrite }) }),

    // Scripts (files in .agent007/scripts/)
    listScripts: () => fetchJson('/api/scripts'),

    // Tool Registry
    listTools: () => fetchJson('/api/tools'),
    discoverTools: () => fetchJson('/api/tools/discover'),
    searchTools: (provider = 'all', q = '', limit = 20) =>
      fetchJson(`/api/tools/search?provider=${encodeURIComponent(provider)}&q=${encodeURIComponent(q)}&limit=${encodeURIComponent(limit)}`),
    importTool: (data) => fetchJson('/api/tools/import', { method: 'POST', body: JSON.stringify(data) }),
    getTool: (name) => fetchJson(`/api/tools/${encodeURIComponent(name)}`),
    saveTool: (data) => fetchJson('/api/tools', { method: 'POST', body: JSON.stringify(data) }),
    deleteTool: (name, scope = 'project') =>
      fetchJson(`/api/tools/${encodeURIComponent(name)}?scope=${encodeURIComponent(scope)}`, { method: 'DELETE' }),
    testTool: (name, args = null) =>
      fetchJson(`/api/tools/${encodeURIComponent(name)}/test`, { method: 'POST', body: JSON.stringify({ args }) }),
    approveTool: (name, scope = 'project', approvedBy = null) =>
      fetchJson(`/api/tools/${encodeURIComponent(name)}/approve`, {
        method: 'POST',
        body: JSON.stringify({ scope, approved_by: approvedBy }),
      }),

    // Skill generation
    generateSkill: (data) => fetchJson('/api/skills/generate', { method: 'POST', body: JSON.stringify(data) }),

    // ETR (Embedded Tool Runtime)
    etrListTools: () => fetchJson('/api/etr/tools'),
    etrCall: (tool, input = {}, compact = true) =>
      fetchJson('/api/etr/call', { method: 'POST', body: JSON.stringify({ tool, input, compact }) }),
    etrCacheStats: () => fetchJson('/api/etr/cache/stats'),
    etrCacheClear: () => fetchJson('/api/etr/cache/clear', { method: 'POST' }),

    // Extensions
    previewExtension: (data) => fetchJson('/api/extensions/preview', { method: 'POST', body: JSON.stringify(data) }),
    installExtension: (data) => fetchJson('/api/extensions/install', { method: 'POST', body: JSON.stringify(data) }),
    listExtensions: () => fetchJson('/api/extensions/list'),

    // MCP Server Registry
    listMcpServers: () => fetchJson('/api/mcp/servers'),
    addMcpServer: (data) => fetchJson('/api/mcp/servers', { method: 'POST', body: JSON.stringify(data) }),
    deleteMcpServer: (name) => fetchJson(`/api/mcp/servers/${encodeURIComponent(name)}`, { method: 'DELETE' }),
    connectMcpServer: (name) => fetchJson(`/api/mcp/servers/${encodeURIComponent(name)}/connect`, { method: 'POST' }),
    approveMcpServer: (name) => fetchJson(`/api/mcp/servers/${encodeURIComponent(name)}/approve`, { method: 'POST' }),
    getMcpServerTools: (name) => fetchJson(`/api/mcp/servers/${encodeURIComponent(name)}/tools`),

    // LSP config
    getLspConfig: () => fetchJson('/api/lsp/config'),
    setLspConfig: (data) => fetchJson('/api/lsp/config', { method: 'POST', body: JSON.stringify(data) }),
    clearLspConfig: () => fetchJson('/api/lsp/config', { method: 'DELETE' }),

    // RAG Sources
    listRagSources: () => fetchJson('/api/rag/sources'),
    addRagSource: (data) => fetchJson('/api/rag/sources', { method: 'POST', body: JSON.stringify(data) }),
    deleteRagSource: (id) => fetchJson(`/api/rag/sources/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    reindexRagSource: (id) => fetchJson(`/api/rag/sources/${encodeURIComponent(id)}/reindex`, { method: 'POST' }),
    queryRag: (q, limit = 5) => fetchJson(`/api/rag/query?q=${encodeURIComponent(q)}&limit=${encodeURIComponent(limit)}`),
  }

  return { api, loading, error, call }
}

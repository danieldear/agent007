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
    getSkill: (trigger) => fetchJson(`/api/skills/detail/${encodeURIComponent(trigger)}`),
    importSkill: (url) => fetchJson('/api/skills/import', { method: 'POST', body: JSON.stringify({ url }) }),
    getRegistry: () => fetchJson('/api/skill-registry'),

    // Workflows
    listWorkflows: () => fetchJson('/api/workflows'),
    getWorkflow: (name) => fetchJson(`/api/workflows/${encodeURIComponent(name)}`),
    saveWorkflow: (data) => fetchJson('/api/workflows', { method: 'POST', body: JSON.stringify(data) }),

    // Workflow validation
    validateWorkflow: (data) => fetchJson('/api/workflows/validate', { method: 'POST', body: JSON.stringify(data) }),

    // Workflow Templates
    listTemplates: () => fetchJson('/api/workflow-templates'),
    getTemplate: (name) => fetchJson(`/api/workflow-templates/${encodeURIComponent(name)}`),

    // Memory
    listMemory: (scope) => fetchJson(`/api/memory/${encodeURIComponent(scope)}`),
    getMemory: (scope, key) => fetch(`/api/memory/${encodeURIComponent(scope)}/${encodeURIComponent(key)}`).then(r => {
      if (!r.ok) throw new Error(`${r.status} ${r.statusText}`)
      return r.text()
    }),

    // Status
    getStatus: () => fetchJson('/api/status'),
    getStats: () => fetchJson('/api/stats'),
    listRuns: () => fetchJson('/api/runs'),
    getRunDetail: (id) => fetchJson(`/api/runs/${encodeURIComponent(id)}`),
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
    exportBundle: (skills, workflows) => {
      const params = new URLSearchParams()
      if (skills?.length) params.set('skills', skills.join(','))
      if (workflows?.length) params.set('workflows', workflows.join(','))
      return fetch(BASE + `/api/bundle/export?${params}`)
    },

    // Sharing — bundle import
    importBundle: (bundle, overwrite) =>
      fetchJson('/api/bundle/import', { method: 'POST', body: JSON.stringify({ bundle, overwrite }) }),
  }

  return { api, loading, error, call }
}

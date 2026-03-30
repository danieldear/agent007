<script setup>
import { ref, onMounted } from 'vue'
import { useApi } from '../composables/useApi.js'

const { api, loading } = useApi()
const personas = ref([])
const showForm = ref(false)
const editTarget = ref(null)

const form = ref({
  name: '',
  description: '',
  preferred_model: 'codex',
  allowed_tools: '',
  system_prompt: '',
})

onMounted(loadPersonas)

async function loadPersonas() {
  const data = await api.listPersonas()
  if (data) personas.value = data
}

function openCreate() {
  editTarget.value = null
  form.value = { name: '', description: '', preferred_model: 'codex', allowed_tools: '', system_prompt: '' }
  showForm.value = true
}

function openEdit(p) {
  editTarget.value = p.name
  form.value = {
    name: p.name,
    description: p.description || '',
    preferred_model: p.preferred_model || 'codex',
    allowed_tools: (p.allowed_tools || []).join(', '),
    system_prompt: p.system_prompt || '',
  }
  showForm.value = true
}

async function savePersona() {
  const payload = {
    ...form.value,
    allowed_tools: form.value.allowed_tools.split(',').map(s => s.trim()).filter(Boolean),
  }
  await api.savePersona(payload)
  showForm.value = false
  await loadPersonas()
}

async function deletePersona(name) {
  if (!confirm(`Delete agent "${name}"?`)) return
  await api.deletePersona(name)
  await loadPersonas()
}
</script>

<template>
  <div class="flex flex-col h-full">
    <div class="p-4 border-b border-base-300 bg-base-200 flex items-center justify-between">
      <h2 class="text-lg font-bold">Agents / Personas</h2>
      <button class="btn btn-sm btn-primary" @click="openCreate">+ New Agent</button>
    </div>

    <div class="flex-1 overflow-auto p-4">
      <!-- Agent cards grid -->
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        <div
          v-for="p in personas"
          :key="p.name"
          class="card bg-base-200 border border-base-300 shadow-sm hover:shadow-md transition-shadow"
        >
          <div class="card-body p-4">
            <div class="flex items-start justify-between">
              <h3 class="card-title text-sm text-primary">{{ p.name }}</h3>
              <div class="flex gap-1">
                <button class="btn btn-ghost btn-xs" @click="openEdit(p)">Edit</button>
                <button class="btn btn-ghost btn-xs text-error" @click="deletePersona(p.name)">Del</button>
              </div>
            </div>
            <p class="text-xs text-base-content/60 mt-1">{{ p.description }}</p>
            <div class="mt-2 flex flex-wrap gap-1">
              <span class="badge badge-xs badge-outline badge-info">{{ p.preferred_model }}</span>
              <span
                v-for="tool in (p.allowed_tools || []).slice(0, 5)"
                :key="tool"
                class="badge badge-xs badge-outline"
              >{{ tool }}</span>
              <span
                v-if="(p.allowed_tools || []).length > 5"
                class="badge badge-xs badge-ghost"
              >+{{ p.allowed_tools.length - 5 }}</span>
            </div>
            <details class="mt-2">
              <summary class="text-xs cursor-pointer text-base-content/40 hover:text-base-content/70">
                System prompt
              </summary>
              <pre class="mt-1 text-xs bg-base-300 p-2 rounded max-h-32 overflow-auto whitespace-pre-wrap">{{ p.system_prompt }}</pre>
            </details>
          </div>
        </div>
      </div>

      <div v-if="!personas.length && !loading" class="text-center text-base-content/40 py-16">
        No agents found. Click "+ New Agent" to create one.
      </div>
    </div>

    <!-- Create/Edit modal -->
    <dialog :open="showForm" class="modal" :class="{ 'modal-open': showForm }">
      <div class="modal-box max-w-2xl bg-base-200">
        <h3 class="font-bold text-lg">{{ editTarget ? `Edit: ${editTarget}` : 'Create New Agent' }}</h3>

        <div class="mt-4 space-y-3">
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Name</span></label>
            <input v-model="form.name" class="input input-sm input-bordered" :disabled="!!editTarget" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Description</span></label>
            <input v-model="form.description" class="input input-sm input-bordered" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Preferred Model</span></label>
            <select v-model="form.preferred_model" class="select select-sm select-bordered">
              <option>codex</option>
              <option>gpt-5.3-codex</option>
              <option>claude</option>
              <option>claude-sonnet-4-6</option>
              <option>ollama</option>
            </select>
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">Allowed Tools (comma-separated)</span></label>
            <input v-model="form.allowed_tools" class="input input-sm input-bordered" placeholder="bash, file_read, file_write" />
          </div>
          <div class="form-control">
            <label class="label"><span class="label-text text-xs">System Prompt</span></label>
            <textarea v-model="form.system_prompt" class="textarea textarea-bordered text-sm font-mono" rows="8" />
          </div>
        </div>

        <div class="modal-action">
          <button class="btn btn-sm btn-ghost" @click="showForm = false">Cancel</button>
          <button class="btn btn-sm btn-primary" @click="savePersona">Save Agent</button>
        </div>
      </div>
      <form method="dialog" class="modal-backdrop"><button @click="showForm = false">close</button></form>
    </dialog>
  </div>
</template>

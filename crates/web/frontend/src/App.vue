<script setup>
import { ref, computed } from 'vue'
import { useWebSocket } from './composables/useWebSocket.js'
import Sidebar from './components/Sidebar.vue'
import DashboardView from './views/DashboardView.vue'
import AgentsView from './views/AgentsView.vue'
import SkillsView from './views/SkillsView.vue'
import WorkflowsView from './views/WorkflowsView.vue'
import ToolsView from './views/ToolsView.vue'
import McpView from './views/McpView.vue'
import LspView from './views/LspView.vue'
import ExtensionsView from './views/ExtensionsView.vue'
import MemoryView from './views/MemoryView.vue'
import SharingView from './views/SharingView.vue'
import HelpView from './views/HelpView.vue'
import RunDetailView from './views/RunDetailView.vue'

const currentView = ref('dashboard')
const selectedRunId = ref(null)
const { connected, events, stats } = useWebSocket()

const views = {
  dashboard: DashboardView,
  agents: AgentsView,
  skills: SkillsView,
  workflows: WorkflowsView,
  tools: ToolsView,
  mcp: McpView,
  lsp: LspView,
  extensions: ExtensionsView,
  memory: MemoryView,
  sharing: SharingView,
  help: HelpView,
  'run-detail': RunDetailView,
}

function openRun(id) {
  selectedRunId.value = id
  currentView.value = 'run-detail'
}

function goBack() {
  currentView.value = 'dashboard'
  selectedRunId.value = null
}

function navigate(view) {
  currentView.value = view
  selectedRunId.value = null
}

const ActiveView = computed(() => views[currentView.value] || DashboardView)
</script>

<template>
  <div class="flex h-screen bg-base-300 text-base-content">
    <Sidebar
      :current="currentView"
      :connected="connected"
      @navigate="navigate"
    />
    <main class="flex-1 overflow-hidden flex flex-col">
      <component
        :is="ActiveView"
        :events="events"
        :connected="connected"
        :stats="stats"
        :run-id="currentView === 'run-detail' ? selectedRunId : undefined"
        @open-run="openRun"
        @go-back="goBack"
      />
    </main>
  </div>
</template>

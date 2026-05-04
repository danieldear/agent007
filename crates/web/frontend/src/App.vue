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
import ExtensionsView from './views/ExtensionsView.vue'
import MemoryView from './views/MemoryView.vue'
import SharingView from './views/SharingView.vue'
import HelpView from './views/HelpView.vue'

const currentView = ref('dashboard')
const { connected, events, stats } = useWebSocket()

const views = {
  dashboard: DashboardView,
  agents: AgentsView,
  skills: SkillsView,
  workflows: WorkflowsView,
  tools: ToolsView,
  mcp: McpView,
  extensions: ExtensionsView,
  memory: MemoryView,
  sharing: SharingView,
  help: HelpView,
}

const ActiveView = computed(() => views[currentView.value] || DashboardView)
</script>

<template>
  <div class="flex h-screen bg-base-300 text-base-content">
    <Sidebar :current="currentView" :connected="connected" @navigate="currentView = $event" />
    <main class="flex-1 overflow-hidden flex flex-col">
      <component :is="ActiveView" :events="events" :connected="connected" :stats="stats" />
    </main>
  </div>
</template>

import { ref, onUnmounted } from 'vue'

export function useWebSocket() {
  const connected = ref(false)
  const events = ref([])
  const stats = ref(null)
  const MAX_EVENTS = 200
  let ws = null
  let timer = null

  function connect() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
    ws = new WebSocket(`${proto}//${location.host}/ws`)

    ws.onopen = () => { connected.value = true }
    ws.onclose = () => {
      connected.value = false
      ws = null
      timer = setTimeout(connect, 2000)
    }
    ws.onerror = () => { if (ws) ws.close() }
    ws.onmessage = (e) => {
      try {
        const msg = JSON.parse(e.data)
        if (msg.type === 'StatusUpdate' && msg.metrics) {
          stats.value = msg.metrics
        } else {
          events.value.push({ ...msg, _ts: new Date().toISOString() })
          if (events.value.length > MAX_EVENTS) events.value.shift()
        }
      } catch (_) {}
    }
  }

  function disconnect() {
    clearTimeout(timer)
    if (ws) { ws.close(); ws = null }
  }

  connect()
  onUnmounted(disconnect)

  return { connected, events, stats, disconnect }
}

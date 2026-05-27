export function graphExplorerPayloadFromResult(result) {
  const payload = result?.output
  if (!payload || typeof payload !== 'object') return payload || null
  return Object.prototype.hasOwnProperty.call(payload, 'output') ? payload.output : payload
}

/// Stable FNV-1a hash used for context reuse and prompt-deduplication stubs.
///
/// This is intentionally small, deterministic, and non-cryptographic. It must
/// remain stable across crates because hosted workflow responses and CLI
/// context rendering both expose the hash in human-visible reuse references.
pub fn stable_context_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

use anyhow::{anyhow, Context, Result};

const KEYCHAIN_SERVICE: &str = "io.agent007.provider-credentials";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderCredential {
    pub id: &'static str,
    pub env_var: &'static str,
}

const PROVIDERS: [ProviderCredential; 2] = [
    ProviderCredential {
        id: "anthropic",
        env_var: "ANTHROPIC_API_KEY",
    },
    ProviderCredential {
        id: "openai",
        env_var: "OPENAI_API_KEY",
    },
];

pub fn provider(id: &str) -> Option<ProviderCredential> {
    PROVIDERS.iter().copied().find(|provider| provider.id == id)
}

pub fn keychain_supported() -> bool {
    cfg!(target_os = "macos")
}

pub fn get(id: &str) -> Option<String> {
    let provider = provider(id)?;
    std::env::var(provider.env_var)
        .ok()
        .filter(|secret| !secret.trim().is_empty())
        .or_else(|| read_keychain(provider.id).ok().flatten())
}

pub fn source(id: &str) -> Option<&'static str> {
    let provider = provider(id)?;
    if std::env::var(provider.env_var)
        .map(|secret| !secret.trim().is_empty())
        .unwrap_or(false)
    {
        Some("environment")
    } else if keychain_contains(provider.id) {
        Some("macOS Keychain")
    } else {
        None
    }
}

pub fn keychain_contains(id: &str) -> bool {
    read_keychain(id).ok().flatten().is_some()
}

pub fn set(id: &str, secret: &str) -> Result<()> {
    let provider = provider(id).ok_or_else(|| anyhow!("unsupported credential provider"))?;
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(anyhow!("API key cannot be empty"));
    }
    if secret.len() > 16 * 1024 {
        return Err(anyhow!("API key exceeds the 16 KiB limit"));
    }
    write_keychain(provider.id, secret)
        .with_context(|| format!("failed to save {} credential", provider.id))?;
    Ok(())
}

pub fn delete(id: &str) -> Result<()> {
    let provider = provider(id).ok_or_else(|| anyhow!("unsupported credential provider"))?;
    delete_keychain(provider.id)
        .with_context(|| format!("failed to remove {} credential", provider.id))?;
    Ok(())
}

#[cfg(all(target_os = "macos", not(test)))]
fn read_keychain(account: &str) -> Result<Option<String>> {
    use security_framework::passwords::{generic_password, PasswordOptions};

    let options = PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, account);
    match generic_password(options) {
        Ok(secret) => String::from_utf8(secret)
            .map(Some)
            .context("stored credential is not valid UTF-8"),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(error).context("macOS Keychain read failed"),
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn read_keychain(_account: &str) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(target_os = "macos")]
fn write_keychain(account: &str, secret: &str) -> Result<()> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        account,
        secret.as_bytes(),
    )
    .context("macOS Keychain write failed")
}

#[cfg(not(target_os = "macos"))]
fn write_keychain(_account: &str, _secret: &str) -> Result<()> {
    Err(anyhow!(
        "secure credential storage is currently available on macOS only"
    ))
}

#[cfg(target_os = "macos")]
fn delete_keychain(account: &str) -> Result<()> {
    match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()),
        Err(error) => Err(error).context("macOS Keychain delete failed"),
    }
}

#[cfg(not(target_os = "macos"))]
fn delete_keychain(_account: &str) -> Result<()> {
    Err(anyhow!(
        "secure credential storage is currently available on macOS only"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_supported_provider_ids() {
        assert_eq!(provider("anthropic").unwrap().env_var, "ANTHROPIC_API_KEY");
        assert_eq!(provider("openai").unwrap().env_var, "OPENAI_API_KEY");
        assert!(provider("hosted-mcp").is_none());
        assert!(provider("../openai").is_none());
    }
}

// crates/git-agent/src/pr.rs
use crate::error::GitAgentError;

#[derive(Debug, PartialEq)]
pub enum Platform {
    GitHub { owner: String, repo: String },
    GitLab { owner: String, repo: String },
    Unknown,
}

/// Detect whether a remote URL points to GitHub or GitLab,
/// and extract the owner/repo from the URL.
pub fn detect_platform(url: &str) -> Platform {
    let clean = url
        .trim_end_matches(".git")
        .replace("git@github.com:", "https://github.com/")
        .replace("git@gitlab.com:", "https://gitlab.com/");

    if let Some(rest) = clean.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() == 2 {
            return Platform::GitHub {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            };
        }
    }
    if let Some(rest) = clean.strip_prefix("https://gitlab.com/") {
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() == 2 {
            return Platform::GitLab {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
            };
        }
    }
    Platform::Unknown
}

/// Send a GitHub PR creation request to the given API URL (injectable for testing).
pub async fn post_github_pr(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    title: &str,
    body: &str,
    head: &str,
    base: &str,
) -> Result<String, GitAgentError> {
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "head": head,
        "base": base,
    });

    let resp = client
        .post(api_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "agent007")
        .json(&payload)
        .send()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(GitAgentError::ApiError(format!(
            "GitHub API returned {}: {}",
            status, text
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    json["html_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| GitAgentError::ApiError("missing html_url in response".into()))
}

/// Send a GitLab MR creation request.
pub async fn post_gitlab_mr(
    client: &reqwest::Client,
    api_url: &str,
    token: &str,
    title: &str,
    body: &str,
    head: &str,
    base: &str,
) -> Result<String, GitAgentError> {
    let payload = serde_json::json!({
        "title": title,
        "description": body,
        "source_branch": head,
        "target_branch": base,
    });

    let resp = client
        .post(api_url)
        .header("PRIVATE-TOKEN", token)
        .header("User-Agent", "agent007")
        .json(&payload)
        .send()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(GitAgentError::ApiError(format!(
            "GitLab API returned {}: {}",
            status, text
        )));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GitAgentError::ApiError(e.to_string()))?;

    json["web_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| GitAgentError::ApiError("missing web_url in response".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_github_platform_from_https_url() {
        let url = "https://github.com/myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(
            matches!(platform, Platform::GitHub { owner, repo } if owner == "myorg" && repo == "myrepo")
        );
    }

    #[test]
    fn detect_github_platform_from_ssh_url() {
        let url = "git@github.com:myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(
            matches!(platform, Platform::GitHub { owner, repo } if owner == "myorg" && repo == "myrepo")
        );
    }

    #[test]
    fn detect_gitlab_platform_from_https_url() {
        let url = "https://gitlab.com/mygroup/myrepo.git";
        let platform = detect_platform(url);
        assert!(
            matches!(platform, Platform::GitLab { owner, repo } if owner == "mygroup" && repo == "myrepo")
        );
    }

    #[test]
    fn detect_unknown_platform_for_other_urls() {
        let url = "https://bitbucket.org/myorg/myrepo.git";
        let platform = detect_platform(url);
        assert!(matches!(platform, Platform::Unknown));
    }

    #[tokio::test]
    async fn create_github_pr_sends_correct_payload() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let response_body = serde_json::json!({
            "html_url": "https://github.com/myorg/myrepo/pull/42"
        });

        Mock::given(method("POST"))
            .and(path("/repos/myorg/myrepo/pulls"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(201).set_body_json(&response_body))
            .mount(&mock_server)
            .await;

        let url = format!("{}/repos/myorg/myrepo/pulls", mock_server.uri());
        let client = reqwest::Client::new();
        let result = post_github_pr(
            &client,
            &url,
            "test-token",
            "Add feature",
            "Description here",
            "feature/add-mdns",
            "main",
        )
        .await;

        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert!(result.unwrap().contains("pull/42"));
    }
}

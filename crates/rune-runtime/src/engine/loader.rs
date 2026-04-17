use std::path::{Path, PathBuf};

use rune_spec::{AgentPackage, ModelsSpec, ToolDescriptor};
use tracing::warn;

use crate::error::RuntimeError;

/// GitHub raw content base URL — skills are fetched from here when not found locally.
/// Pattern: `{BASE}/{owner}/{repo}/HEAD/{skill_name}/SKILL.md`
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";

/// skills.sh URL prefix. Skills hosted here are stored on GitHub and resolved
/// via [`GITHUB_RAW_BASE`] using the `owner/repo/skill-name` path segment.
const SKILLS_SH_PREFIX: &str = "https://skills.sh/";

/// SkillsMP URL prefix. Skills published on the SkillsMP marketplace are stored
/// on GitHub and resolved via [`GITHUB_RAW_BASE`] using the `owner/repo/skill-name`
/// path segment.
const SKILLSMP_PREFIX: &str = "https://skillsmp.com/";

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // --- stub ---

    #[test]
    fn stub_sets_correct_agent_name() {
        let plan = ExecutionPlan::stub("my-agent");
        assert_eq!(plan.agent_name, "my-agent");
    }

    #[test]
    fn stub_has_sensible_defaults() {
        let plan = ExecutionPlan::stub("test");
        assert_eq!(plan.max_steps, 10);
        assert_eq!(plan.timeout_ms, 30_000);
        assert!(!plan.instructions.is_empty());
        assert!(!plan.default_model.is_empty());
    }

    #[test]
    fn stub_has_bridge_network() {
        let plan = ExecutionPlan::stub("test");
        assert_eq!(plan.networks, vec!["bridge"]);
    }

    #[test]
    fn stub_has_empty_tools_and_toolset() {
        let plan = ExecutionPlan::stub("test");
        assert!(plan.tools.is_empty());
        assert!(plan.toolset.is_empty());
    }

    #[test]
    fn stub_models_spec_is_default() {
        let plan = ExecutionPlan::stub("test");
        assert!(plan.models.providers.is_empty());
    }

    // --- from_dir ---

    fn write_minimal_agent(dir: &std::path::Path) {
        std::fs::write(
            dir.join("Runefile"),
            r"name: test-agent
version: 0.1.0
instructions: You are a test agent.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
",
        )
        .unwrap();
    }

    #[test]
    fn from_dir_loads_minimal_package() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_agent(dir.path());

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert_eq!(plan.agent_name, "test-agent");
        assert_eq!(plan.max_steps, 20); // spec.yaml default
        assert_eq!(plan.timeout_ms, 30_000);
        assert_eq!(plan.networks, vec!["bridge"]);
    }

    #[test]
    fn from_dir_missing_dir_returns_spec_error() {
        let result = ExecutionPlan::from_dir(std::path::Path::new("/nonexistent/agent"));
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("Spec error") || msg.contains("IO error"));
    }

    #[test]
    fn from_dir_toolset_merges_spec_and_tools_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            r"name: agent
version: 0.1.0
instructions: x
default_model: d
models:
  model_mapping:
    d: claude-sonnet-4-6
toolset:
  - rune@shell
",
        )
        .unwrap();

        let tools_dir = dir.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(tools_dir.join("search.yaml"), "name: my_search\n").unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert!(plan.toolset.contains(&"rune@shell".to_string()));
        assert!(plan.toolset.contains(&"my_search".to_string()));
    }

    #[test]
    fn from_dir_skills_are_appended_to_instructions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            r"name: a
version: 0.1.0
instructions: Base.
default_model: d
models:
  model_mapping:
    d: claude-sonnet-4-6
skills:
  - owner/repo/my-skill
",
        )
        .unwrap();

        let skill_dir = dir.path().join("skills/owner/repo/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Extra skill knowledge.").unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert!(plan.instructions.contains("Base."));
        assert!(plan.instructions.contains("Extra skill knowledge."));
    }

    // --- github_skill_url ---

    #[test]
    fn github_skill_url_builds_correct_url() {
        let url =
            github_skill_url_with_base("anthropics/claude-code/frontend-design", GITHUB_RAW_BASE)
                .unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/anthropics/claude-code/HEAD/frontend-design/SKILL.md"
        );
    }

    #[test]
    fn github_skill_url_requires_three_segments() {
        assert!(github_skill_url_with_base("owner/repo", GITHUB_RAW_BASE).is_none());
        assert!(github_skill_url_with_base("just-one", GITHUB_RAW_BASE).is_none());
    }

    #[test]
    fn github_skill_url_skill_name_with_hyphens() {
        let url =
            github_skill_url_with_base("vercel-labs/agent-skills/find-skills", GITHUB_RAW_BASE)
                .unwrap();
        assert!(url.contains("/vercel-labs/agent-skills/HEAD/find-skills/SKILL.md"));
    }

    #[test]
    fn fallback_skills_dir_url_inserts_skills_segment() {
        let url =
            "https://raw.githubusercontent.com/vercel-labs/agent-skills/HEAD/find-skills/SKILL.md";
        let fallback = fallback_skills_dir_url(url).unwrap();
        assert_eq!(
            fallback,
            "https://raw.githubusercontent.com/vercel-labs/agent-skills/HEAD/skills/find-skills/SKILL.md"
        );
    }

    #[test]
    fn fallback_skills_dir_url_ignored_for_non_github_raw_or_existing_skills_path() {
        assert!(fallback_skills_dir_url("https://example.com/a/b/c/SKILL.md").is_none());
        assert!(fallback_skills_dir_url("https://raw.githubusercontent.com/vercel-labs/agent-skills/HEAD/skills/find-skills/SKILL.md").is_none());
    }

    // --- resolve_skill_url ---

    #[test]
    fn resolve_skill_url_short_form_uses_github() {
        let url = resolve_skill_url("anthropics/claude-code/frontend-design").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/anthropics/claude-code/HEAD/frontend-design/SKILL.md"
        );
    }

    #[test]
    fn resolve_skill_url_skills_sh_prefix_uses_github() {
        let url =
            resolve_skill_url("https://skills.sh/anthropics/claude-code/frontend-design").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/anthropics/claude-code/HEAD/frontend-design/SKILL.md"
        );
    }

    #[test]
    fn resolve_skill_url_skillsmp_prefix_uses_github() {
        let url =
            resolve_skill_url("https://skillsmp.com/vercel-labs/agent-skills/find-skills").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/vercel-labs/agent-skills/HEAD/find-skills/SKILL.md"
        );
    }

    #[test]
    fn resolve_skill_url_generic_https_returned_as_is() {
        let raw = "https://example.com/my-org/my-repo/my-skill/SKILL.md";
        let url = resolve_skill_url(raw).unwrap();
        assert_eq!(url, raw);
    }

    #[test]
    fn resolve_skill_url_short_form_missing_segments_returns_none() {
        assert!(resolve_skill_url("owner/repo").is_none());
        assert!(resolve_skill_url("just-one").is_none());
    }

    #[test]
    fn resolve_skill_url_skills_sh_missing_segments_returns_none() {
        // https://skills.sh/owner/repo — missing skill-name segment
        assert!(resolve_skill_url("https://skills.sh/owner/repo").is_none());
    }

    #[test]
    fn resolve_skill_url_skillsmp_missing_segments_returns_none() {
        assert!(resolve_skill_url("https://skillsmp.com/owner/repo").is_none());
    }

    // --- from_dir_async: local skills still work ---

    #[tokio::test]
    async fn from_dir_async_local_skill_injected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            r"name: a
version: 0.1.0
instructions: Base.
default_model: d
models:
  model_mapping:
    d: claude-sonnet-4-6
skills:
  - owner/repo/my-skill
",
        )
        .unwrap();
        let skill_dir = dir.path().join("skills/owner/repo/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Local skill content.").unwrap();

        let http = reqwest::Client::new();
        let plan = ExecutionPlan::from_dir_async(dir.path(), &http)
            .await
            .unwrap();
        assert!(plan.instructions.contains("Base."));
        assert!(plan.instructions.contains("Local skill content."));
    }

    #[test]
    fn from_dir_with_runefile_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            r"name: runefile-agent
version: 0.2.0
instructions: From Runefile.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
",
        )
        .unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert_eq!(plan.agent_name, "runefile-agent");
    }

    fn start_mock_http_server(
        routes: Vec<(&'static str, u16, &'static str)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for _ in 0..routes.len() {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");

                let (status, body) = routes
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, s, b)| (*s, *b))
                    .unwrap_or((404, "not-found"));

                let status_text = if status == 200 { "OK" } else { "Not Found" };
                let resp = format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn fetch_missing_skills_primary_url_success() {
        let (base, handle) = start_mock_http_server(vec![(
            "/owner/repo/HEAD/my-skill/SKILL.md",
            200,
            "Remote skill content.",
        )]);
        let http = reqwest::Client::new();
        let missing = vec!["owner/repo/my-skill".to_string()];

        let content = fetch_missing_skills_with_base(&http, &missing, &base).await;
        handle.join().unwrap();

        assert_eq!(content, "Remote skill content.");
    }

    #[tokio::test]
    async fn fetch_missing_skills_uses_fallback_skills_dir_on_primary_404() {
        let (base, handle) = start_mock_http_server(vec![
            ("/owner/repo/HEAD/my-skill/SKILL.md", 404, "not-found"),
            (
                "/owner/repo/HEAD/skills/my-skill/SKILL.md",
                200,
                "Fallback skill content.",
            ),
        ]);
        let http = reqwest::Client::new();
        let missing = vec!["owner/repo/my-skill".to_string()];

        let content = fetch_missing_skills_with_base(&http, &missing, &base).await;
        handle.join().unwrap();

        assert_eq!(content, "Fallback skill content.");
    }

    #[tokio::test]
    async fn fetch_missing_skills_keeps_empty_when_primary_and_fallback_fail() {
        let (base, handle) = start_mock_http_server(vec![
            ("/owner/repo/HEAD/my-skill/SKILL.md", 404, "not-found"),
            (
                "/owner/repo/HEAD/skills/my-skill/SKILL.md",
                404,
                "not-found",
            ),
        ]);
        let http = reqwest::Client::new();
        let missing = vec!["owner/repo/my-skill".to_string()];

        let content = fetch_missing_skills_with_base(&http, &missing, &base).await;
        handle.join().unwrap();

        assert!(content.is_empty());
    }
}

/// In-memory execution plan built from an agent package.
pub struct ExecutionPlan {
    pub agent_name: String,
    pub instructions: String,
    pub default_model: String,
    pub max_steps: u32,
    pub timeout_ms: u64,
    pub tools: Vec<ToolDescriptor>,
    pub agent_dir: PathBuf,
    pub models: ModelsSpec,
    pub toolset: Vec<String>,
    /// rune-network memberships for this agent (default: ["bridge"]).
    pub networks: Vec<String>,
}

fn skill_markdown_path(agent_dir: &Path, skill_ref: &str) -> PathBuf {
    let mut p = agent_dir.join("skills");
    for seg in skill_ref.split('/') {
        if seg.is_empty() {
            continue;
        }
        p.push(seg);
    }
    p.join("SKILL.md")
}

/// Returns `(local_markdown_concat, missing_refs)` for remote fetch.
fn collect_local_skills(agent_dir: &Path, skill_refs: &[String]) -> (String, Vec<String>) {
    let mut found: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for r in skill_refs {
        let path = skill_markdown_path(agent_dir, r);
        match std::fs::read_to_string(&path) {
            Ok(text) => found.push(text.trim().to_string()),
            Err(_) => missing.push(r.clone()),
        }
    }
    (found.join("\n\n"), missing)
}

fn load_tools_from_agent_dir(agent_dir: &Path) -> Result<Vec<ToolDescriptor>, RuntimeError> {
    let dir = agent_dir.join("tools");
    if !dir.is_dir() {
        return Ok(vec![]);
    }
    ToolDescriptor::load_dir(&dir).map_err(|e| RuntimeError::Spec(e.to_string()))
}

impl ExecutionPlan {
    /// Load from a local agent directory containing a `Runefile`.
    pub fn from_dir(agent_dir: &Path) -> Result<Self, RuntimeError> {
        let pkg = AgentPackage::load(agent_dir).map_err(|e| RuntimeError::Spec(e.to_string()))?;

        let tools = load_tools_from_agent_dir(agent_dir)?;

        let mut toolset: Vec<String> = pkg.spec.toolset.clone();
        for t in &tools {
            if !toolset.contains(&t.name) {
                toolset.push(t.name.clone());
            }
        }

        let (local_skills, _) = collect_local_skills(agent_dir, &pkg.spec.skills);
        let instructions = if local_skills.is_empty() {
            pkg.spec.instructions.clone()
        } else {
            format!("{}\n\n{}", pkg.spec.instructions.trim_end(), local_skills)
        };

        Ok(Self {
            agent_name: pkg.spec.name,
            instructions,
            default_model: pkg.spec.default_model,
            max_steps: pkg.spec.max_steps,
            timeout_ms: pkg.spec.timeout_ms,
            tools,
            agent_dir: agent_dir.to_path_buf(),
            models: pkg.spec.models.clone(),
            toolset,
            networks: pkg.spec.networks.clone(),
        })
    }

    /// Like [`from_dir`] but also fetches skills that are declared in the Runefile
    /// yet missing from the local `skills/` directory. Skills are retrieved from
    /// GitHub raw content using the `owner/repo/skill-name` path as:
    ///
    /// ```text
    /// https://raw.githubusercontent.com/<owner>/<repo>/HEAD/<skill-name>/SKILL.md
    /// ```
    ///
    /// This is the canonical storage location for skills published on SkillsMP
    /// (<https://skillsmp.com>). Fetch failures are logged and silently skipped so
    /// the agent can still start with partial skill coverage.
    pub async fn from_dir_async(
        agent_dir: &Path,
        http: &reqwest::Client,
    ) -> Result<Self, RuntimeError> {
        let pkg = AgentPackage::load(agent_dir).map_err(|e| RuntimeError::Spec(e.to_string()))?;

        let tools = load_tools_from_agent_dir(agent_dir)?;

        let mut toolset: Vec<String> = pkg.spec.toolset.clone();
        for t in &tools {
            if !toolset.contains(&t.name) {
                toolset.push(t.name.clone());
            }
        }

        let (local_skills, missing) = collect_local_skills(agent_dir, &pkg.spec.skills);
        let remote = fetch_missing_skills(http, &missing).await;

        let skill_block = match (local_skills.is_empty(), remote.is_empty()) {
            (true, true) => String::new(),
            (false, true) => local_skills,
            (true, false) => remote,
            (false, false) => format!("{local_skills}\n\n{remote}"),
        };

        let instructions = if skill_block.is_empty() {
            pkg.spec.instructions.clone()
        } else {
            format!("{}\n\n{}", pkg.spec.instructions.trim_end(), skill_block)
        };

        Ok(Self {
            agent_name: pkg.spec.name,
            instructions,
            default_model: pkg.spec.default_model,
            max_steps: pkg.spec.max_steps,
            timeout_ms: pkg.spec.timeout_ms,
            tools,
            agent_dir: agent_dir.to_path_buf(),
            models: pkg.spec.models.clone(),
            toolset,
            networks: pkg.spec.networks.clone(),
        })
    }

    /// Minimal stub plan — used when the agent package is not locally available
    /// (e.g. during Phase 3 before OCI fetch is implemented).
    pub fn stub(agent_name: impl Into<String>) -> Self {
        Self {
            agent_name: agent_name.into(),
            instructions: "You are a helpful assistant.".into(),
            default_model: "claude-sonnet-4-6".into(),
            max_steps: 10,
            timeout_ms: 30_000,
            tools: vec![],
            agent_dir: PathBuf::new(),
            models: ModelsSpec::default(),
            toolset: vec![],
            networks: vec!["bridge".into()],
        }
    }
}

/// Fetch SKILL.md content for skills not found locally.
///
/// Supported `skill_ref` formats:
/// - `owner/repo/skill-name` — fetched from GitHub raw content
/// - `https://skills.sh/<owner>/<repo>/<skill-name>` — path extracted and fetched from GitHub
/// - `https://skillsmp.com/<owner>/<repo>/<skill-name>` — path extracted and fetched from GitHub
/// - Any other `https://` URL — fetched directly (the URL is used as-is)
///
/// Failures (network, 404, etc.) are logged and skipped.
async fn fetch_missing_skills(http: &reqwest::Client, missing: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for skill_ref in missing {
        match resolve_skill_url(skill_ref) {
            Some(url) => match http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(text) => parts.push(text.trim().to_string()),
                    Err(e) => warn!("Failed to read skill body for '{skill_ref}': {e}"),
                },
                Ok(resp) => {
                    if resp.status().as_u16() == 404 {
                        if let Some(fallback_url) = fallback_skills_dir_url(&url) {
                            match http.get(&fallback_url).send().await {
                                Ok(fallback_resp) if fallback_resp.status().is_success() => {
                                    match fallback_resp.text().await {
                                        Ok(text) => {
                                            parts.push(text.trim().to_string());
                                            continue;
                                        }
                                        Err(e) => {
                                            warn!(
                                                    "Failed to read fallback skill body for '{skill_ref}': {e}"
                                                );
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    }
                    warn!(
                        "Skill '{skill_ref}' not found remotely (HTTP {})",
                        resp.status()
                    );
                }
                Err(e) => {
                    warn!("Failed to fetch skill '{skill_ref}' from {url}: {e}");
                }
            },
            None => {
                warn!("Cannot resolve skill ref '{skill_ref}': unsupported format, skipping remote fetch");
            }
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
async fn fetch_missing_skills_with_base(
    http: &reqwest::Client,
    missing: &[String],
    github_raw_base: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    for skill_ref in missing {
        match resolve_skill_url_with_base(skill_ref, github_raw_base) {
            Some(url) => match http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(text) => parts.push(text.trim().to_string()),
                    Err(e) => warn!("Failed to read skill body for '{skill_ref}': {e}"),
                },
                Ok(resp) => {
                    if resp.status().as_u16() == 404 {
                        if let Some(fallback_url) =
                            fallback_skills_dir_url_with_base(&url, github_raw_base)
                        {
                            match http.get(&fallback_url).send().await {
                                Ok(fallback_resp) if fallback_resp.status().is_success() => {
                                    match fallback_resp.text().await {
                                        Ok(text) => {
                                            parts.push(text.trim().to_string());
                                            continue;
                                        }
                                        Err(e) => {
                                            warn!(
                                                    "Failed to read fallback skill body for '{skill_ref}': {e}"
                                                );
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(_) => {}
                            }
                        }
                    }
                    warn!(
                        "Skill '{skill_ref}' not found remotely (HTTP {})",
                        resp.status()
                    );
                }
                Err(e) => {
                    warn!("Failed to fetch skill '{skill_ref}' from {url}: {e}");
                }
            },
            None => {
                warn!("Cannot resolve skill ref '{skill_ref}': unsupported format, skipping remote fetch");
            }
        }
    }
    parts.join("\n\n")
}

/// Resolve a skill ref to the URL from which SKILL.md should be fetched.
///
/// | Input format | Resolution |
/// |---|---|
/// | `owner/repo/skill-name` | GitHub raw content URL |
/// | `https://skills.sh/<owner>/<repo>/<skill-name>` | GitHub raw content URL (path extracted) |
/// | `https://skillsmp.com/<owner>/<repo>/<skill-name>` | GitHub raw content URL (path extracted) |
/// | any other `https://` URL | returned as-is |
///
/// Returns `None` when the short `owner/repo/skill-name` format is used but the
/// ref doesn't have exactly three `/`-separated segments.
fn resolve_skill_url(skill_ref: &str) -> Option<String> {
    resolve_skill_url_with_base(skill_ref, GITHUB_RAW_BASE)
}

fn resolve_skill_url_with_base(skill_ref: &str, github_raw_base: &str) -> Option<String> {
    if let Some(path) = skill_ref.strip_prefix(SKILLS_SH_PREFIX) {
        // https://skills.sh/<owner>/<repo>/<skill-name>
        github_skill_url_with_base(path, github_raw_base)
    } else if let Some(path) = skill_ref.strip_prefix(SKILLSMP_PREFIX) {
        // https://skillsmp.com/<owner>/<repo>/<skill-name>
        github_skill_url_with_base(path, github_raw_base)
    } else if skill_ref.starts_with("https://") {
        // Generic URL — caller knows the exact location of the SKILL.md file.
        Some(skill_ref.to_string())
    } else {
        // Short form: owner/repo/skill-name
        github_skill_url_with_base(skill_ref, github_raw_base)
    }
}

fn github_skill_url_with_base(skill_ref: &str, github_raw_base: &str) -> Option<String> {
    let parts: Vec<&str> = skill_ref.splitn(3, '/').collect();
    if parts.len() != 3 {
        return None;
    }
    let (owner, repo, skill_name) = (parts[0], parts[1], parts[2]);
    Some(format!(
        "{github_raw_base}/{owner}/{repo}/HEAD/{skill_name}/SKILL.md"
    ))
}

fn fallback_skills_dir_url(url: &str) -> Option<String> {
    fallback_skills_dir_url_with_base(url, GITHUB_RAW_BASE)
}

fn fallback_skills_dir_url_with_base(url: &str, github_raw_base: &str) -> Option<String> {
    let skill_suffix = "/SKILL.md";
    if !url.starts_with(github_raw_base) || !url.ends_with(skill_suffix) || url.contains("/skills/")
    {
        return None;
    }
    let head_idx = url.find("/HEAD/")?;
    let before = &url[..head_idx + "/HEAD/".len()];
    let after = &url[head_idx + "/HEAD/".len()..];
    Some(format!("{before}skills/{after}"))
}

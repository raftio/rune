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
            "name: test-agent\nversion: 0.1.0\ninstructions: You are a test agent.\ndefault_model: default\nruntime: {}\nmodels: {}\n",
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
            "name: agent\nversion: 0.1.0\ninstructions: x\ndefault_model: d\ntoolset:\n  - rune@shell\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();

        let tools_dir = dir.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(
            tools_dir.join("search.yaml"),
            "name: my_search\n",
        )
        .unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert!(plan.toolset.contains(&"rune@shell".to_string()));
        assert!(plan.toolset.contains(&"my_search".to_string()));
    }

    #[test]
    fn from_dir_skills_are_appended_to_instructions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\nskills:\n  - owner/repo/my-skill\n",
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
        let url = github_skill_url("anthropics/claude-code/frontend-design").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/anthropics/claude-code/HEAD/frontend-design/SKILL.md"
        );
    }

    #[test]
    fn github_skill_url_requires_three_segments() {
        assert!(github_skill_url("owner/repo").is_none());
        assert!(github_skill_url("just-one").is_none());
    }

    #[test]
    fn github_skill_url_skill_name_with_hyphens() {
        let url = github_skill_url("vercel-labs/agent-skills/find-skills").unwrap();
        assert!(url.contains("/vercel-labs/agent-skills/HEAD/find-skills/SKILL.md"));
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
        let url = resolve_skill_url("https://skills.sh/anthropics/claude-code/frontend-design").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/anthropics/claude-code/HEAD/frontend-design/SKILL.md"
        );
    }

    #[test]
    fn resolve_skill_url_skillsmp_prefix_uses_github() {
        let url = resolve_skill_url("https://skillsmp.com/vercel-labs/agent-skills/find-skills").unwrap();
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
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\nskills:\n  - owner/repo/my-skill\n",
        )
        .unwrap();
        let skill_dir = dir.path().join("skills/owner/repo/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Local skill content.").unwrap();

        let http = reqwest::Client::new();
        let plan = ExecutionPlan::from_dir_async(dir.path(), &http).await.unwrap();
        assert!(plan.instructions.contains("Base."));
        assert!(plan.instructions.contains("Local skill content."));
    }

    #[test]
    fn from_dir_with_runefile_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: runefile-agent\nversion: 0.2.0\ninstructions: From Runefile.\ndefault_model: default\nruntime:\n  concurrency_limit: 5\nmodels: {}\n",
        )
        .unwrap();

        let plan = ExecutionPlan::from_dir(dir.path()).unwrap();
        assert_eq!(plan.agent_name, "runefile-agent");
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

impl ExecutionPlan {
    /// Load from a local agent directory containing a `Runefile`.
    pub fn from_dir(agent_dir: &Path) -> Result<Self, RuntimeError> {
        let pkg = AgentPackage::load(agent_dir)
            .map_err(|e| RuntimeError::Spec(e.to_string()))?;
        let mut toolset: Vec<String> = pkg.spec.toolset.clone();
        for t in &pkg.tools {
            if !toolset.contains(&t.name) {
                toolset.push(t.name.clone());
            }
        }
        let instructions = if pkg.skill_instructions.is_empty() {
            pkg.spec.instructions
        } else {
            format!("{}\n\n{}", pkg.spec.instructions.trim_end(), pkg.skill_instructions)
        };

        Ok(Self {
            agent_name: pkg.spec.name,
            instructions,
            default_model: pkg.spec.default_model,
            max_steps: pkg.spec.max_steps,
            timeout_ms: pkg.spec.timeout_ms,
            tools: pkg.tools,
            agent_dir: agent_dir.to_path_buf(),
            models: pkg.models,
            toolset,
            networks: pkg.spec.networks,
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
    pub async fn from_dir_async(agent_dir: &Path, http: &reqwest::Client) -> Result<Self, RuntimeError> {
        let pkg = AgentPackage::load(agent_dir)
            .map_err(|e| RuntimeError::Spec(e.to_string()))?;

        let mut toolset: Vec<String> = pkg.spec.toolset.clone();
        for t in &pkg.tools {
            if !toolset.contains(&t.name) {
                toolset.push(t.name.clone());
            }
        }

        // Fetch skills that weren't found in the local skills/ directory.
        let remote_parts = fetch_missing_skills(http, &pkg.missing_skills).await;

        let skill_instructions = if remote_parts.is_empty() {
            pkg.skill_instructions
        } else if pkg.skill_instructions.is_empty() {
            remote_parts
        } else {
            format!("{}\n\n{}", pkg.skill_instructions, remote_parts)
        };

        let instructions = if skill_instructions.is_empty() {
            pkg.spec.instructions
        } else {
            format!("{}\n\n{}", pkg.spec.instructions.trim_end(), skill_instructions)
        };

        Ok(Self {
            agent_name: pkg.spec.name,
            instructions,
            default_model: pkg.spec.default_model,
            max_steps: pkg.spec.max_steps,
            timeout_ms: pkg.spec.timeout_ms,
            tools: pkg.tools,
            agent_dir: agent_dir.to_path_buf(),
            models: pkg.models,
            toolset,
            networks: pkg.spec.networks,
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
            Some(url) => {
                match http.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.text().await {
                            Ok(text) => parts.push(text.trim().to_string()),
                            Err(e) => warn!("Failed to read skill body for '{skill_ref}': {e}"),
                        }
                    }
                    Ok(resp) => {
                        warn!("Skill '{skill_ref}' not found remotely (HTTP {})", resp.status());
                    }
                    Err(e) => {
                        warn!("Failed to fetch skill '{skill_ref}' from {url}: {e}");
                    }
                }
            }
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
    if let Some(path) = skill_ref.strip_prefix(SKILLS_SH_PREFIX) {
        // https://skills.sh/<owner>/<repo>/<skill-name>
        github_skill_url(path)
    } else if let Some(path) = skill_ref.strip_prefix(SKILLSMP_PREFIX) {
        // https://skillsmp.com/<owner>/<repo>/<skill-name>
        github_skill_url(path)
    } else if skill_ref.starts_with("https://") {
        // Generic URL — caller knows the exact location of the SKILL.md file.
        Some(skill_ref.to_string())
    } else {
        // Short form: owner/repo/skill-name
        github_skill_url(skill_ref)
    }
}

/// Build a GitHub raw content URL for a skill ref `owner/repo/skill-name`.
/// Returns `None` if the ref doesn't have exactly three path segments.
fn github_skill_url(skill_ref: &str) -> Option<String> {
    let parts: Vec<&str> = skill_ref.splitn(3, '/').collect();
    if parts.len() != 3 {
        return None;
    }
    let (owner, repo, skill_name) = (parts[0], parts[1], parts[2]);
    Some(format!(
        "{GITHUB_RAW_BASE}/{owner}/{repo}/HEAD/{skill_name}/SKILL.md"
    ))
}

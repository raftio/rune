pub mod agent;
pub mod models;
pub mod runefile;
pub mod tool;
pub mod error;

pub use agent::AgentSpec;
pub use models::ModelsSpec;
pub use runefile::Runefile;
pub use tool::{ToolDescriptor, ToolRuntime};
pub use error::SpecError;

use std::path::Path;

/// Full agent package loaded from an agent directory containing a `Runefile`.
pub struct AgentPackage {
    pub spec: AgentSpec,
    pub models: ModelsSpec,
    pub tools: Vec<ToolDescriptor>,
}

impl AgentPackage {
    pub fn load(agent_dir: &Path) -> Result<Self, SpecError> {
        let runefile_path = agent_dir.join("Runefile");
        let rf = Runefile::load(&runefile_path)?;
        let (spec, models) = (rf.spec, rf.models);

        let tools_dir = agent_dir.join("tools");
        let mut tools = if tools_dir.exists() {
            ToolDescriptor::load_dir(&tools_dir)?
        } else {
            vec![]
        };

        // Auto-inject built-in tool descriptors for `rune@` entries in toolset
        for name in &spec.toolset {
            if name.starts_with("rune@") && !tools.iter().any(|t| &t.name == name) {
                tools.push(ToolDescriptor::builtin(name));
            }
        }

        Ok(Self { spec, models, tools })
    }
}


/// Reads SKILL.md files for each declared skill reference.
/// Skills are expected at `skills/<owner>/<repo>/<skill-name>/SKILL.md` relative
/// to the agent directory, matching the layout produced by `npx skills add`.
///
/// URL-format refs (`https://skills.sh/…`, `https://skillsmp.com/…`, or any
/// `https://` URL) are skipped for local lookup and added directly to
/// `missing_refs` so the runtime can fetch them remotely.
///
/// Returns `(found_content, missing_refs)`:
/// - `found_content`: concatenated content of all locally resolved skills
/// - `missing_refs`: skill refs not found locally — the runtime can fetch these remotely
fn load_skills(agent_dir: &Path, skills: &[String]) -> (String, Vec<String>) {
    let skills_dir = agent_dir.join("skills");
    let mut parts: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for skill_ref in skills {
        // URL-format refs have no local representation — defer to remote fetch.
        if skill_ref.starts_with("https://") {
            missing.push(skill_ref.clone());
            continue;
        }
        let skill_path = skills_dir.join(skill_ref).join("SKILL.md");
        match std::fs::read_to_string(&skill_path) {
            Ok(content) => parts.push(content.trim().to_string()),
            Err(_) => missing.push(skill_ref.clone()),
        }
    }
    (parts.join("\n\n"), missing)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn write_minimal_runefile(dir: &Path) {
        std::fs::write(
            dir.join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();
    }

    #[test]
    fn load_builtin_tools_auto_injected_for_rune_at_toolset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nruntime: {}\nmodels: {}\ntoolset:\n  - rune@memory-store\n  - rune@memory-recall\n",
        )
        .unwrap();

        let pkg = AgentPackage::load(dir.path()).unwrap();
        let tool_names: Vec<&str> = pkg.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"rune@memory-store"));
        assert!(tool_names.contains(&"rune@memory-recall"));
        // Auto-injected descriptors are marked as builtin
        assert!(pkg.tools.iter().all(|t| t.is_builtin()));
    }

    #[test]
    fn load_invalid_workflow_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("workflow.yaml"), "not: valid: yaml: [").unwrap();

        let result = AgentPackage::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_invalid_tool_yaml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nruntime: {}\nmodels: {}\n",
        )
        .unwrap();
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(tools_dir.join("bad.yaml"), "name: [unclosed").unwrap();

        let result = AgentPackage::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_tools_dir_with_valid_tool_loaded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: x\ndefault_model: d\nruntime: {}\nmodels: {}\ntoolset:\n  - my_search\n",
        )
        .unwrap();
        let tools_dir = dir.path().join("tools");
        std::fs::create_dir(&tools_dir).unwrap();
        std::fs::write(tools_dir.join("search.yaml"), "name: my_search\n").unwrap();

        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.tools.iter().any(|t| t.name == "my_search"));
        assert!(pkg.spec.toolset.contains(&"my_search".to_string()));
    }
}

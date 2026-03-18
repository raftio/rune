pub mod agent;
pub mod compose;
pub mod runtime;
pub mod models;
pub mod runefile;
pub mod tool;
pub mod workflow;
pub mod error;

pub use agent::AgentSpec;
pub use runtime::RuntimeSpec;
pub use models::ModelsSpec;
pub use runefile::Runefile;
pub use tool::{ToolDescriptor, ToolRuntime};
pub use workflow::WorkflowSpec;
pub use compose::ComposeSpec;
pub use error::SpecError;

use std::path::Path;

/// Full agent package loaded from an agent directory containing a `Runefile`.
pub struct AgentPackage {
    pub spec: AgentSpec,
    pub runtime: RuntimeSpec,
    pub models: ModelsSpec,
    pub tools: Vec<ToolDescriptor>,
    pub workflow: Option<WorkflowSpec>,
    /// Concatenated content of locally resolved skills, ready to append to instructions.
    pub skill_instructions: String,
    /// Skills declared in the Runefile but not found in the local `skills/` directory.
    /// The runtime can fetch these remotely (e.g. from GitHub via SkillsMP).
    pub missing_skills: Vec<String>,
}

impl AgentPackage {
    pub fn load(agent_dir: &Path) -> Result<Self, SpecError> {
        let runefile_path = agent_dir.join("Runefile");
        let rf = Runefile::load(&runefile_path)?;
        let (spec, runtime, models) = (rf.spec, rf.runtime, rf.models);

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

        let workflow_path = agent_dir.join("workflow.yaml");
        let workflow = if workflow_path.exists() {
            Some(WorkflowSpec::load(&workflow_path)?)
        } else {
            None
        };

        // Load skills from `skills/<owner>/<repo>/<skill-name>/SKILL.md`
        // Install skills with: npx skills add owner/repo/skill-name
        let (skill_instructions, missing_skills) = load_skills(agent_dir, &spec.skills);

        Ok(Self { spec, runtime, models, tools, workflow, skill_instructions, missing_skills })
    }
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
    fn load_no_skills_gives_empty_skill_instructions() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_runefile(dir.path());
        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.skill_instructions.is_empty());
    }

    #[test]
    fn load_skills_injects_skill_file_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\nskills:\n  - owner/repo/my-skill\n",
        )
        .unwrap();

        let skill_dir = dir.path().join("skills/owner/repo/my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# My Skill\nDo something useful.").unwrap();

        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.skill_instructions.contains("My Skill"));
        assert!(pkg.skill_instructions.contains("Do something useful."));
    }

    #[test]
    fn load_skills_missing_skill_file_tracked_in_missing_skills() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\nskills:\n  - owner/repo/nonexistent\n",
        )
        .unwrap();

        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.skill_instructions.is_empty());
        assert_eq!(pkg.missing_skills, vec!["owner/repo/nonexistent"]);
    }

    #[test]
    fn load_no_skills_has_empty_missing_skills() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_runefile(dir.path());
        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.missing_skills.is_empty());
    }

    #[test]
    fn load_skills_multiple_skills_are_joined() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Runefile"),
            "name: a\nversion: 0.1.0\ninstructions: Base.\ndefault_model: d\nruntime: {}\nmodels: {}\nskills:\n  - owner/repo/skill-a\n  - owner/repo/skill-b\n",
        )
        .unwrap();

        for name in &["skill-a", "skill-b"] {
            let d = dir.path().join(format!("skills/owner/repo/{name}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("SKILL.md"), format!("Content of {name}.")).unwrap();
        }

        let pkg = AgentPackage::load(dir.path()).unwrap();
        assert!(pkg.skill_instructions.contains("Content of skill-a."));
        assert!(pkg.skill_instructions.contains("Content of skill-b."));
    }
}

/// Reads SKILL.md files for each declared skill reference.
/// Skills are expected at `skills/<owner>/<repo>/<skill-name>/SKILL.md` relative
/// to the agent directory, matching the layout produced by `npx skills add`.
///
/// Returns `(found_content, missing_refs)`:
/// - `found_content`: concatenated content of all locally resolved skills
/// - `missing_refs`: skill refs not found locally — the runtime can fetch these remotely
fn load_skills(agent_dir: &Path, skills: &[String]) -> (String, Vec<String>) {
    let skills_dir = agent_dir.join("skills");
    let mut parts: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for skill_ref in skills {
        let skill_path = skills_dir.join(skill_ref).join("SKILL.md");
        match std::fs::read_to_string(&skill_path) {
            Ok(content) => parts.push(content.trim().to_string()),
            Err(_) => missing.push(skill_ref.clone()),
        }
    }
    (parts.join("\n\n"), missing)
}

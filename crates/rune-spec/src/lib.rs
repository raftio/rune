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

        Ok(Self { spec, runtime, models, tools, workflow })
    }
}

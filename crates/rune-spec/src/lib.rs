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
}

impl AgentPackage {
    pub fn load(agent_dir: &Path) -> Result<Self, SpecError> {
        let runefile_path = agent_dir.join("Runefile");
        let rf = Runefile::load(&runefile_path)?;
        let (spec, models) = (rf.spec, rf.models);

        Ok(Self { spec, models })
    }
}
#[cfg(test)]
mod tests {

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

    }
}

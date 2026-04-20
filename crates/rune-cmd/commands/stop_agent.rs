use anyhow::Result;

use crate::cli::AgentStopArgs;

pub async fn exec(args: AgentStopArgs) -> Result<()> {
    crate::commands::agent::stop_by_name(args).await
}

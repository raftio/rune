use anyhow::Result;

use crate::cli::AgentRmArgs;

pub async fn exec(args: AgentRmArgs) -> Result<()> {
    crate::commands::agent::rm_by_name(args).await
}

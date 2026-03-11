use anyhow::{bail, Result};

use crate::cli::{
    ClusterAddLearnerArgs, ClusterChangeMembershipArgs, ClusterInitArgs, ClusterStatusArgs,
};

pub async fn init(args: ClusterInitArgs) -> Result<()> {
    println!("Initializing single-node cluster at {} ...", args.addr);
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/cluster/init", args.addr))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("Cluster initialized successfully.");
    } else {
        bail!("Failed to initialize cluster: {}", resp.text().await?);
    }
    Ok(())
}

pub async fn add_learner(args: ClusterAddLearnerArgs) -> Result<()> {
    println!(
        "Adding learner node {} ({}) via leader at {} ...",
        args.node_id, args.node_addr, args.leader_addr
    );
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/cluster/add-learner", args.leader_addr))
        .json(&serde_json::json!({
            "node_id": args.node_id,
            "addr": args.node_addr,
        }))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("Learner {} added.", args.node_id);
    } else {
        bail!("Failed to add learner: {}", resp.text().await?);
    }
    Ok(())
}

pub async fn change_membership(args: ClusterChangeMembershipArgs) -> Result<()> {
    println!(
        "Changing membership to {:?} via leader at {} ...",
        args.members, args.leader_addr
    );
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/cluster/change-membership", args.leader_addr))
        .json(&serde_json::json!({ "members": args.members }))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("Membership changed to {:?}.", args.members);
    } else {
        bail!("Failed to change membership: {}", resp.text().await?);
    }
    Ok(())
}

pub async fn status(args: ClusterStatusArgs) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/cluster/status", args.addr))
        .send()
        .await?;

    if resp.status().is_success() {
        let body: serde_json::Value = resp.json().await?;
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        bail!("Failed to get cluster status: {}", resp.text().await?);
    }
    Ok(())
}

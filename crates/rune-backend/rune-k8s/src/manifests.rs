use std::collections::BTreeMap;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::autoscaling::v2::{
    HorizontalPodAutoscaler, HorizontalPodAutoscalerSpec, MetricSpec, MetricTarget,
    ResourceMetricSource,
};
use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EnvVar, PodSpec, PodTemplateSpec, Service, ServicePort, ServiceSpec,
    Volume, VolumeMount,
};
use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

const MANAGED_BY: &str = "rune";
const AGENT_PORT: i32 = 8080;

pub struct ManifestParams<'a> {
    pub deployment_id: &'a str,
    pub replica_id: &'a str,
    pub image: &'a str,
    pub namespace: &'a str,
    pub min_replicas: i32,
    pub max_replicas: i32,
    pub config_ref: Option<&'a str>,
    pub secret_ref: Option<&'a str>,
    pub env_json: Option<&'a str>,
}

fn rune_labels(deployment_id: &str, replica_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.kubernetes.io/managed-by".into(), MANAGED_BY.into()),
        ("rune.io/deployment-id".into(), deployment_id.into()),
        ("rune.io/replica-id".into(), replica_id.into()),
    ])
}

fn selector_labels(replica_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("app.kubernetes.io/managed-by".into(), MANAGED_BY.into()),
        ("rune.io/replica-id".into(), replica_id.into()),
    ])
}

fn resource_name(replica_id: &str) -> String {
    format!("rune-{}", &replica_id[..8.min(replica_id.len())])
}

fn parse_env_vars(env_json: Option<&str>) -> Vec<EnvVar> {
    let Some(raw) = env_json else { return vec![] };
    let map: BTreeMap<String, String> = serde_json::from_str(raw).unwrap_or_default();
    map.into_iter()
        .map(|(k, v)| EnvVar {
            name: k,
            value: Some(v),
            ..Default::default()
        })
        .collect()
}

pub fn deployment_manifest(p: &ManifestParams) -> Deployment {
    let name = resource_name(p.replica_id);
    let labels = rune_labels(p.deployment_id, p.replica_id);
    let sel_labels = selector_labels(p.replica_id);
    let env = parse_env_vars(p.env_json);

    let mut volumes = Vec::new();
    let mut volume_mounts = Vec::new();

    if let Some(config) = p.config_ref {
        volumes.push(Volume {
            name: "agent-config".into(),
            config_map: Some(k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                name: config.into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: "agent-config".into(),
            mount_path: "/agent/config".into(),
            read_only: Some(true),
            ..Default::default()
        });
    }

    if let Some(secret) = p.secret_ref {
        volumes.push(Volume {
            name: "agent-secrets".into(),
            secret: Some(k8s_openapi::api::core::v1::SecretVolumeSource {
                secret_name: Some(secret.into()),
                ..Default::default()
            }),
            ..Default::default()
        });
        volume_mounts.push(VolumeMount {
            name: "agent-secrets".into(),
            mount_path: "/agent/secrets".into(),
            read_only: Some(true),
            ..Default::default()
        });
    }

    Deployment {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(p.namespace.into()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(k8s_openapi::api::apps::v1::DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(sel_labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "agent".into(),
                        image: Some(p.image.into()),
                        command: Some(vec![
                            "agent-runtime".into(),
                            "--agent-dir".into(),
                            "/agent".into(),
                        ]),
                        ports: Some(vec![ContainerPort {
                            container_port: AGENT_PORT,
                            name: Some("http".into()),
                            ..Default::default()
                        }]),
                        env: if env.is_empty() { None } else { Some(env) },
                        volume_mounts: if volume_mounts.is_empty() {
                            None
                        } else {
                            Some(volume_mounts)
                        },
                        ..Default::default()
                    }],
                    volumes: if volumes.is_empty() {
                        None
                    } else {
                        Some(volumes)
                    },
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn service_manifest(p: &ManifestParams) -> Service {
    let name = resource_name(p.replica_id);
    let labels = rune_labels(p.deployment_id, p.replica_id);
    let sel_labels = selector_labels(p.replica_id);

    Service {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(p.namespace.into()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(sel_labels),
            ports: Some(vec![ServicePort {
                port: AGENT_PORT,
                target_port: Some(IntOrString::String("http".into())),
                name: Some("http".into()),
                ..Default::default()
            }]),
            cluster_ip: Some("None".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn pdb_manifest(p: &ManifestParams) -> PodDisruptionBudget {
    let name = resource_name(p.replica_id);
    let labels = rune_labels(p.deployment_id, p.replica_id);
    let sel_labels = selector_labels(p.replica_id);

    PodDisruptionBudget {
        metadata: ObjectMeta {
            name: Some(format!("{name}-pdb")),
            namespace: Some(p.namespace.into()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(PodDisruptionBudgetSpec {
            min_available: Some(IntOrString::Int(1)),
            selector: Some(LabelSelector {
                match_labels: Some(sel_labels),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn hpa_manifest(p: &ManifestParams) -> HorizontalPodAutoscaler {
    let name = resource_name(p.replica_id);
    let labels = rune_labels(p.deployment_id, p.replica_id);

    HorizontalPodAutoscaler {
        metadata: ObjectMeta {
            name: Some(format!("{name}-hpa")),
            namespace: Some(p.namespace.into()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(HorizontalPodAutoscalerSpec {
            scale_target_ref: k8s_openapi::api::autoscaling::v2::CrossVersionObjectReference {
                api_version: Some("apps/v1".into()),
                kind: "Deployment".into(),
                name: name,
            },
            min_replicas: Some(p.min_replicas),
            max_replicas: p.max_replicas,
            metrics: Some(vec![MetricSpec {
                type_: "Resource".into(),
                resource: Some(ResourceMetricSource {
                    name: "cpu".into(),
                    target: MetricTarget {
                        type_: "Utilization".into(),
                        average_utilization: Some(80),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

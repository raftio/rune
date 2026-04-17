use std::io::{Cursor, Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rune_spec::AgentPackage;
use tar::Archive;
use tar::Builder;

use crate::{
    extract_and_load_package, extract_to_temp, materialize_agent_bundle, pack_agent_dir, verify,
    verify_dir, ArtifactError, PackOptions,
};

fn minimal_agent(dir: &std::path::Path) {
    std::fs::write(
        dir.join("Runefile"),
        r"name: test-a
version: 0.1.0
instructions: Hello.
default_model: default
models:
  model_mapping:
    default: claude-sonnet-4-6
",
    )
    .unwrap();
}

fn append_tar_entry<W: Write>(tar: &mut Builder<W>, path: &str, bytes: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).unwrap();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, bytes).unwrap();
}

/// Decompress, flip one byte in `agent/Runefile` payload, recompress. Manifest still lists old hash.
fn corrupt_runefile_in_artifact(tgz: &[u8]) -> Vec<u8> {
    let decoder = GzDecoder::new(Cursor::new(tgz));
    let mut archive = Archive::new(decoder);
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if path == "agent/Runefile" && !data.is_empty() {
            data[0] ^= 0x01;
        }
        entries.push((path, data));
    }
    let mut out = Vec::new();
    {
        let gz = GzEncoder::new(&mut out, Compression::default());
        let mut tar = Builder::new(gz);
        for (path, data) in entries {
            append_tar_entry(&mut tar, &path, &data);
        }
        tar.finish().unwrap();
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();
    }
    out
}

#[test]
fn pack_verify_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    let summary = pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();
    assert_eq!(summary.artifact_sha256.len(), 64);

    let m = verify(Cursor::new(&buf)).unwrap();
    assert_eq!(m.agent_name, "test-a");
    assert_eq!(m.model, "claude-sonnet-4-6");
    assert_eq!(m.format, crate::FORMAT_V1);
    assert_eq!(m.initiative.as_deref(), Some(crate::INITIATIVE_OPEN_AGENT));
    assert!(m.files.iter().any(|f| f.path == "Runefile"));
}

#[test]
fn extract_and_load_matches_name() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    let (_tmp, pkg) = extract_and_load_package(Cursor::new(&buf)).unwrap();
    assert_eq!(pkg.spec.name, "test-a");
    assert_eq!(pkg.resolved_model().unwrap(), "claude-sonnet-4-6");
}

#[test]
fn verify_fails_on_corrupted_gzip() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    assert!(buf.len() > 16);
    buf[10] ^= 0xFF;
    assert!(verify(Cursor::new(&buf)).is_err());
}

#[test]
fn verify_fails_hash_mismatch_when_file_tampered() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    let bad = corrupt_runefile_in_artifact(&buf);
    let err = verify(Cursor::new(&bad)).unwrap_err();
    match err {
        ArtifactError::HashMismatch { path, .. } => assert_eq!(path, "Runefile"),
        other => panic!("expected HashMismatch, got {other:?}"),
    }
}

#[test]
fn extract_to_temp_has_runefile() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    let (_tmp, agent_root) = extract_to_temp(Cursor::new(&buf)).unwrap();
    assert!(agent_root.join("Runefile").is_file());
    let pkg = AgentPackage::load(&agent_root).unwrap();
    assert_eq!(pkg.spec.name, "test-a");
    assert_eq!(pkg.spec.version, "0.1.0");
}

#[test]
fn pack_optional_tag_in_manifest() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(
        dir.path(),
        &mut buf,
        PackOptions {
            tag: Some("v1-rc".to_string()),
        },
    )
    .unwrap();

    let m = verify(Cursor::new(&buf)).unwrap();
    assert_eq!(m.tag.as_deref(), Some("v1-rc"));
    assert_eq!(m.initiative.as_deref(), Some(crate::INITIATIVE_OPEN_AGENT));
}

#[test]
fn materialize_and_verify_dir_matches_pack_manifest_fields() {
    let src = tempfile::tempdir().unwrap();
    minimal_agent(src.path());
    let root = tempfile::tempdir().unwrap();

    materialize_agent_bundle(
        src.path(),
        root.path(),
        PackOptions {
            tag: Some("v2".to_string()),
        },
    )
    .unwrap();

    let m = verify_dir(root.path()).unwrap();
    assert_eq!(m.agent_name, "test-a");
    assert_eq!(m.tag.as_deref(), Some("v2"));
    assert_eq!(m.initiative.as_deref(), Some(crate::INITIATIVE_OPEN_AGENT));

    let m_list = crate::read_manifest_dir(root.path()).unwrap();
    assert_eq!(m_list.agent_name, m.agent_name);
}

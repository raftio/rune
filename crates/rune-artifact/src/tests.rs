use std::io::{Cursor, Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rune_spec::AgentPackage;
use tar::Archive;
use tar::Builder;

use crate::{
    extract_and_load_package, extract_to_temp, pack_agent_dir, verify, ArtifactError, PackOptions,
};

fn minimal_agent(dir: &std::path::Path) {
    std::fs::write(
        dir.join("Runefile"),
        "name: test-a\nversion: 0.1.0\ninstructions: Hello.\ndefault_model: d\nruntime: {}\nmodels: {}\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("tools")).unwrap();
    std::fs::write(dir.join("tools").join("t.yaml"), "name: t\n").unwrap();
    std::fs::create_dir_all(dir.join("skills/o/r/skill-x")).unwrap();
    std::fs::write(
        dir.join("skills/o/r/skill-x/SKILL.md"),
        "Skill body.\n",
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
    assert_eq!(m.format, crate::FORMAT_V1);
    assert_eq!(m.initiative.as_deref(), Some(crate::INITIATIVE_OPEN_AGENT));
    assert!(m.files.iter().any(|f| f.path == "Runefile"));
    assert!(m.files.iter().any(|f| f.path.starts_with("tools/")));
}

#[test]
fn extract_and_load_matches_name() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    let (_tmp, pkg) = extract_and_load_package(Cursor::new(&buf)).unwrap();
    assert_eq!(pkg.spec.name, "test-a");
}

#[test]
fn verify_fails_on_corrupted_gzip() {
    let dir = tempfile::tempdir().unwrap();
    minimal_agent(dir.path());

    let mut buf = Vec::new();
    pack_agent_dir(dir.path(), &mut buf, PackOptions::default()).unwrap();

    // Flip payload after gzip header (0x1f 0x8b) so decompression fails or output is garbage.
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

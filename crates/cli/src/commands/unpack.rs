// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit unpack` runner.
//!
//! Inverse of `invoicekit pack`. Reads a `.ikb` evidence bundle
//! and writes every artefact to disk under an output directory,
//! preserving the artefact ids as path components.
//!
//! Operators use this to inspect a bundle's contents (e.g. read
//! the canonical JSON, diff the rendered PDF, look at the
//! manifest) without writing their own unpacker. The reverse of
//! the produce → verify → replay loop: produce locally, ship as
//! a bundle, then unpack on the receiver to inspect.
//!
//! Exit codes:
//!
//! * `0` — bundle unpacked.
//! * `1` — write failure mid-extract (artefact-level IO error).
//! * `2` — usage error (bad args, unreadable file, malformed
//!   bundle, output directory exists and `--force` not set).

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use invoicekit_evidence::{unpack, MANIFEST_ARTEFACT_ID};

/// Run `invoicekit unpack`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let bytes = match fs::read(&parsed.bundle) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("unpack: cannot read {}: {err}", parsed.bundle.display());
            return ExitCode::from(2);
        }
    };

    let bundle = match unpack(&bytes) {
        Ok(b) => b,
        Err(err) => {
            eprintln!(
                "unpack: {} is not a valid evidence bundle: {err}",
                parsed.bundle.display()
            );
            return ExitCode::from(2);
        }
    };

    // Output directory policy: refuse to clobber a non-empty
    // directory unless --force is set. An empty existing dir is
    // fine (operators often `mkdir foo && invoicekit unpack
    // bundle.ikb foo`).
    if parsed.output_dir.exists() {
        let non_empty = fs::read_dir(&parsed.output_dir).is_ok_and(|mut it| it.next().is_some());
        if non_empty && !parsed.force {
            eprintln!(
                "unpack: {} is not empty — pass --force to overwrite",
                parsed.output_dir.display()
            );
            return ExitCode::from(2);
        }
    } else if let Err(err) = fs::create_dir_all(&parsed.output_dir) {
        eprintln!(
            "unpack: cannot create {}: {err}",
            parsed.output_dir.display()
        );
        return ExitCode::from(2);
    }

    // `unpack` strips the manifest out of `bundle.artefacts`
    // into the typed `bundle.manifest` field. Re-serialise it
    // so operators see manifest.json on disk alongside the
    // payload artefacts.
    let manifest_bytes = match serde_json::to_vec_pretty(&bundle.manifest) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("unpack: manifest serialise failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(code) = write_artefact(&parsed.output_dir, MANIFEST_ARTEFACT_ID, &manifest_bytes) {
        return code;
    }

    for (id, bytes) in &bundle.artefacts {
        if let Err(code) = write_artefact(&parsed.output_dir, id, bytes) {
            return code;
        }
    }

    // +1 for the manifest we serialised above.
    eprintln!(
        "unpack: wrote {} artefacts to {}",
        bundle.artefacts.len() + 1,
        parsed.output_dir.display()
    );
    ExitCode::SUCCESS
}

fn write_artefact(out_dir: &Path, id: &str, bytes: &[u8]) -> Result<(), ExitCode> {
    let rel = match safe_relative_path(id) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("unpack: rejecting artefact id {id:?}: {msg}");
            return Err(ExitCode::from(2));
        }
    };
    let path = out_dir.join(rel);
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("unpack: cannot mkdir {}: {err}", parent.display());
            return Err(ExitCode::FAILURE);
        }
    }
    if let Err(err) = fs::write(&path, bytes) {
        eprintln!("unpack: cannot write {}: {err}", path.display());
        return Err(ExitCode::FAILURE);
    }
    Ok(())
}

#[derive(Debug)]
struct Args {
    bundle: PathBuf,
    output_dir: PathBuf,
    force: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut bundle: Option<PathBuf> = None;
    let mut output_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "--help" | "-h" => return Err(usage_help()),
            "--force" => {
                force = true;
                i += 1;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unpack: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if bundle.is_none() {
                    bundle = Some(PathBuf::from(positional));
                } else if output_dir.is_none() {
                    output_dir = Some(PathBuf::from(positional));
                } else {
                    return Err(format!(
                        "unpack: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                i += 1;
            }
        }
    }
    let bundle =
        bundle.ok_or_else(|| format!("unpack: <bundle.ikb> required\n\n{}", usage_help()))?;
    let output_dir =
        output_dir.ok_or_else(|| format!("unpack: <output-dir> required\n\n{}", usage_help()))?;
    Ok(Args {
        bundle,
        output_dir,
        force,
    })
}

fn usage_help() -> String {
    "usage: invoicekit unpack <bundle.ikb> <output-dir> [--force]\n\nExtract every artefact from a .ikb evidence bundle into <output-dir>, preserving the artefact ids as path components. Refuses to overwrite a non-empty <output-dir> unless --force is set.".to_owned()
}

/// Reject artefact ids that would escape `output-dir` via
/// absolute paths or `..` components. Bundles authored by
/// well-behaved producers never contain such ids, but the
/// container format does not enforce this on its own, so we
/// guard here at the write boundary.
fn safe_relative_path(id: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(id);
    if candidate.is_absolute() {
        return Err("absolute path".to_owned());
    }
    for c in candidate.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err("`..` component".to_owned()),
            Component::RootDir | Component::Prefix(_) => return Err("rooted path".to_owned()),
        }
    }
    Ok(candidate.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
    use std::collections::BTreeMap;

    fn sample_bundle() -> Vec<u8> {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-U-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(
            &artefacts,
            "tenant-unpack",
            "trace-unpack",
            "2026-05-28T03:55:00Z",
        );
        pack(&EvidenceBundle {
            manifest,
            artefacts,
        })
        .unwrap()
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_output_returns_usage_error() {
        let code = run(&["bundle.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_bundle_file_returns_usage_error() {
        let out = tempfile::tempdir().unwrap();
        let code = run(&[
            "/tmp/this/file/does/not/exist.ikb".to_owned(),
            out.path().join("dst").to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_malformed_bundle_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.ikb");
        fs::write(&bad, b"not a bundle").unwrap();
        let out = dir.path().join("dst");
        let code = run(&[
            bad.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_valid_bundle_writes_all_artefacts() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("sample.ikb");
        fs::write(&bundle_path, sample_bundle()).unwrap();
        let out = dir.path().join("out");
        let code = run(&[
            bundle_path.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
        let canon = fs::read_to_string(out.join("canonical.json")).unwrap();
        assert!(canon.contains("INV-U-1"));
        let ubl = fs::read_to_string(out.join("formats/ubl.xml")).unwrap();
        assert!(ubl.contains("<Invoice/>"));
        // Manifest is also written.
        assert!(out.join("manifest.json").is_file());
    }

    #[test]
    fn run_refuses_to_clobber_non_empty_output_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("sample.ikb");
        fs::write(&bundle_path, sample_bundle()).unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("preexisting.txt"), b"keep me").unwrap();
        let code = run(&[
            bundle_path.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_accepts_existing_empty_output() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("sample.ikb");
        fs::write(&bundle_path, sample_bundle()).unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        let code = run(&[
            bundle_path.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_overwrites_non_empty_output_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("sample.ikb");
        fs::write(&bundle_path, sample_bundle()).unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("stale.txt"), b"old").unwrap();
        let code = run(&[
            bundle_path.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
            "--force".to_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
        // Stale file still there (we don't `rm -rf` the dir;
        // --force just stops the safety bail-out).
        assert!(out.join("stale.txt").is_file());
        // Plus the new artefacts.
        assert!(out.join("canonical.json").is_file());
    }

    #[test]
    fn safe_relative_path_rejects_absolute_and_parent() {
        assert!(safe_relative_path("/etc/passwd").is_err());
        assert!(safe_relative_path("../escape").is_err());
        assert!(safe_relative_path("a/../b").is_err());
        assert!(safe_relative_path("ok/path.json").is_ok());
        assert!(safe_relative_path("nested/deeper/file.xml").is_ok());
    }
}

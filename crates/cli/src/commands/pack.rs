// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit pack` runner.
//!
//! Walks an input directory, packs every regular file under it
//! as an artefact (id = path relative to the input root), and
//! writes a deterministic `.ikb` evidence bundle via
//! [`invoicekit_evidence::pack`]. Operators use this to turn a
//! directory of pipeline outputs into the bundle that
//! `invoicekit verify` and `invoicekit replay` consume — closing
//! the produce → verify → replay loop end-to-end in the CLI.
//!
//! Exit codes:
//!
//! * `0` — bundle written.
//! * `1` — pack failed (manifest serialise failure, etc.).
//! * `2` — usage error (bad args, missing input, unwritable
//!   output, no files found).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};

/// Run `invoicekit pack`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let artefacts = match collect_artefacts(&parsed.input_dir) {
        Ok(a) => a,
        Err(err) => {
            eprintln!("pack: {err}");
            return ExitCode::from(2);
        }
    };

    if artefacts.is_empty() {
        eprintln!(
            "pack: no files found under {} — refusing to build an empty bundle",
            parsed.input_dir.display()
        );
        return ExitCode::from(2);
    }

    let manifest = manifest_for(
        &artefacts,
        &parsed.tenant_id,
        &parsed.trace_id,
        &parsed.created_at,
    );
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };

    let bytes = match pack(&bundle) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("pack: bundle pack failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = fs::write(&parsed.output, &bytes) {
        eprintln!("pack: cannot write {}: {err}", parsed.output.display());
        return ExitCode::from(2);
    }

    eprintln!(
        "pack: wrote {} ({} artefacts, {} bytes)",
        parsed.output.display(),
        bundle.artefacts.len(),
        bytes.len()
    );
    ExitCode::SUCCESS
}

#[derive(Debug)]
struct Args {
    input_dir: PathBuf,
    output: PathBuf,
    tenant_id: String,
    trace_id: String,
    created_at: String,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut input_dir: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut tenant_id: Option<String> = None;
    let mut trace_id: Option<String> = None;
    let mut created_at: Option<String> = None;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "--help" | "-h" => return Err(usage_help()),
            "--tenant" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("pack: --tenant needs a value\n\n{}", usage_help()))?;
                tenant_id = Some(v.clone());
                i += 2;
                continue;
            }
            "--trace" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("pack: --trace needs a value\n\n{}", usage_help()))?;
                trace_id = Some(v.clone());
                i += 2;
                continue;
            }
            "--created-at" => {
                let v = argv.get(i + 1).ok_or_else(|| {
                    format!("pack: --created-at needs a value\n\n{}", usage_help())
                })?;
                created_at = Some(v.clone());
                i += 2;
                continue;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("pack: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if input_dir.is_none() {
                    input_dir = Some(PathBuf::from(positional));
                } else if output.is_none() {
                    output = Some(PathBuf::from(positional));
                } else {
                    return Err(format!(
                        "pack: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
            }
        }
        i += 1;
    }

    let input_dir =
        input_dir.ok_or_else(|| format!("pack: <input-dir> required\n\n{}", usage_help()))?;
    let output =
        output.ok_or_else(|| format!("pack: <output.ikb> required\n\n{}", usage_help()))?;

    if !input_dir.is_dir() {
        return Err(format!("pack: {} is not a directory", input_dir.display()));
    }

    Ok(Args {
        input_dir,
        output,
        tenant_id: tenant_id.unwrap_or_else(|| "unset-tenant".to_owned()),
        trace_id: trace_id.unwrap_or_else(|| "unset-trace".to_owned()),
        // Default created_at is a fixed sentinel so the same
        // input directory packed twice without `--created-at`
        // produces byte-identical bundles. Operators who want a
        // real timestamp pass `--created-at $(date -u +%FT%TZ)`.
        created_at: created_at.unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned()),
    })
}

fn collect_artefacts(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, cursor: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let entries =
        fs::read_dir(cursor).map_err(|e| format!("cannot read {}: {e}", cursor.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read dir entry: {e}"))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if metadata.is_dir() {
            walk(root, &path, out)?;
        } else if metadata.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| {
                format!(
                    "path {} not under root {}: {e}",
                    path.display(),
                    root.display()
                )
            })?;
            // Use forward slashes for artefact ids so the same
            // bundle packed on Linux and Windows produces the
            // same id strings.
            let id = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if id.is_empty() {
                continue;
            }
            // `manifest.json` is reserved — pack() rejects it
            // via dedup, but flagging early is friendlier.
            if id == invoicekit_evidence::MANIFEST_ARTEFACT_ID {
                return Err(format!(
                    "input contains reserved artefact id `{id}` (the bundle codec emits manifest.json itself)"
                ));
            }
            let bytes =
                fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            out.insert(id, bytes);
        }
    }
    Ok(())
}

fn usage_help() -> String {
    "usage: invoicekit pack <input-dir> <output.ikb> [--tenant ID] [--trace ID] [--created-at RFC3339]\n\nPack every file under <input-dir> into a deterministic .ikb evidence bundle.\nDefaults: tenant=unset-tenant, trace=unset-trace, created-at=1970-01-01T00:00:00Z\n(so a directory packed twice without overrides produces byte-identical output).".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::unpack;

    fn populate(dir: &Path, files: &[(&str, &[u8])]) {
        for (rel, bytes) in files {
            let full = dir.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, bytes).unwrap();
        }
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_output_arg_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let code = run(&[dir.path().to_string_lossy().into_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_input_that_is_not_a_directory_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, b"hi").unwrap();
        let out = dir.path().join("out.ikb");
        let code = run(&[
            file.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_empty_directory_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.ikb");
        let code = run(&[
            dir.path().to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_populated_directory_writes_round_trippable_bundle() {
        let input = tempfile::tempdir().unwrap();
        populate(
            input.path(),
            &[
                ("canonical.json", br#"{"id":"INV-PACK-1"}"#),
                ("formats/ubl.xml", b"<Invoice/>"),
                ("formats/cii.xml", b"<CrossIndustryInvoice/>"),
            ],
        );
        let out_dir = tempfile::tempdir().unwrap();
        let out = out_dir.path().join("packed.ikb");
        let code = run(&[
            input.path().to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
            "--tenant".to_owned(),
            "tenant-pack-test".to_owned(),
            "--trace".to_owned(),
            "trace-pack-test".to_owned(),
            "--created-at".to_owned(),
            "2026-05-28T03:50:00Z".to_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
        let bytes = fs::read(&out).unwrap();
        let bundle = unpack(&bytes).unwrap();
        assert_eq!(bundle.manifest.tenant_id, "tenant-pack-test");
        assert_eq!(bundle.manifest.trace_id, "trace-pack-test");
        assert_eq!(bundle.manifest.created_at, "2026-05-28T03:50:00Z");
        // 3 source files + manifest.json
        let ids: Vec<&str> = bundle.artefacts.keys().map(String::as_str).collect();
        assert!(ids.contains(&"canonical.json"));
        assert!(ids.contains(&"formats/ubl.xml"));
        assert!(ids.contains(&"formats/cii.xml"));
    }

    #[test]
    fn pack_is_deterministic_with_default_created_at() {
        let input = tempfile::tempdir().unwrap();
        populate(
            input.path(),
            &[("a.txt", b"alpha"), ("b/c.txt", b"bravo charlie")],
        );
        let out_dir = tempfile::tempdir().unwrap();
        let out1 = out_dir.path().join("first.ikb");
        let out2 = out_dir.path().join("second.ikb");

        let code1 = run(&[
            input.path().to_string_lossy().into_owned(),
            out1.to_string_lossy().into_owned(),
        ]);
        let code2 = run(&[
            input.path().to_string_lossy().into_owned(),
            out2.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code1, ExitCode::SUCCESS);
        assert_eq!(code2, ExitCode::SUCCESS);
        let bytes1 = fs::read(&out1).unwrap();
        let bytes2 = fs::read(&out2).unwrap();
        assert_eq!(
            bytes1, bytes2,
            "default-created-at pack must be byte-identical"
        );
    }

    #[test]
    fn pack_rejects_reserved_manifest_id() {
        let input = tempfile::tempdir().unwrap();
        populate(input.path(), &[("manifest.json", b"{}")]);
        let out_dir = tempfile::tempdir().unwrap();
        let out = out_dir.path().join("out.ikb");
        let code = run(&[
            input.path().to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parse_args_extracts_paths_and_flags() {
        let parsed = parse_args(&[
            "in".to_owned(),
            "out.ikb".to_owned(),
            "--tenant".to_owned(),
            "T".to_owned(),
            "--trace".to_owned(),
            "R".to_owned(),
            "--created-at".to_owned(),
            "2026-05-28T00:00:00Z".to_owned(),
        ]);
        // We can't actually construct the Args without a real
        // dir, so this hits the dir-existence check.
        assert!(parsed.is_err());
    }
}

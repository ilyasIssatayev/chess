use chess_evaluation::{LoadError, load_manifest, validate_manifest_collection};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths: Vec<_> = env::args_os().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: manifest-check <manifest.json> [manifest.json ...]");
        return ExitCode::from(2);
    }

    let mut failed = false;
    let mut loaded = Vec::new();
    for path in paths {
        match load_manifest(&path) {
            Ok(manifest) => {
                loaded.push((path, manifest));
            }
            Err(LoadError::Validation(errors)) => {
                failed = true;
                eprintln!(
                    "{}: invalid ({} error(s))",
                    path.to_string_lossy(),
                    errors.len()
                );
                for error in errors {
                    eprintln!("  - {error}");
                }
            }
            Err(error) => {
                failed = true;
                eprintln!("{}: {error}", path.to_string_lossy());
            }
        }
    }

    let manifests: Vec<_> = loaded.iter().map(|(_, manifest)| manifest).collect();
    if let Err(errors) = validate_manifest_collection(&manifests) {
        failed = true;
        eprintln!("manifest collection: invalid ({} error(s))", errors.len());
        for error in errors {
            eprintln!("  - {error}");
        }
    } else {
        for (path, manifest) in &loaded {
            println!(
                "{}: valid ({} session(s))",
                path.to_string_lossy(),
                manifest.sessions.len()
            );
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

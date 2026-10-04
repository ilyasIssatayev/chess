//! Consistent SQLite + evidence snapshots; restore only into a new directory.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use storage::Store;
use vision_inference::runtime::sha256;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    file: String,
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    files: Vec<Entry>,
}
pub fn create(store: &Store, data: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.exists(), "Backup destination already exists");
    let stage = destination.with_extension(format!("staging-{}", uuid::Uuid::now_v7()));
    fs::create_dir_all(&stage)?;
    let result = (|| -> Result<()> {
        store.backup_to(&stage.join("games.sqlite"))?;
        fs::create_dir(stage.join("evidence"))?;
        let mut files = vec![entry(&stage, "games.sqlite")?];
        for game in store.games()? {
            for evidence in store.evidence(game.id)? {
                let name = format!("evidence/{}.jpg", evidence.id);
                match fs::read(data.join(&name)) {
                    Ok(bytes) if sha256(&bytes) == evidence.sha256 => {
                        fs::write(stage.join(&name), &bytes)?;
                        files.push(entry(&stage, &name)?);
                    }
                    // Missing evidence remains recorded as missing in the database and backup.
                    _ => {}
                }
            }
        }
        fs::write(
            stage.join("backup.json"),
            serde_json::to_vec_pretty(&Manifest { version: 1, files })?,
        )?;
        validate(&stage)?;
        fs::rename(&stage, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
fn entry(root: &Path, name: &str) -> Result<Entry> {
    let bytes = fs::read(root.join(name))?;
    Ok(Entry {
        file: name.into(),
        bytes: bytes.len() as u64,
        sha256: sha256(&bytes),
    })
}
fn safe(name: &str) -> bool {
    name == "games.sqlite"
        || name
            .strip_prefix("evidence/")
            .and_then(|n| n.strip_suffix(".jpg"))
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok() && !id.contains('/'))
}
fn validate(root: &Path) -> Result<Manifest> {
    ensure!(
        fs::metadata(root.join("backup.json"))?.len() <= 4 * 1024 * 1024,
        "Oversized backup manifest"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(root.join("backup.json"))?)?;
    ensure!(
        manifest.version == 1
            && manifest
                .files
                .iter()
                .filter(|e| e.file == "games.sqlite")
                .count()
                == 1,
        "Invalid backup manifest"
    );
    let mut seen = std::collections::HashSet::new();
    let mut total = 0u64;
    for e in &manifest.files {
        ensure!(
            safe(&e.file) && seen.insert(&e.file),
            "Unsafe or duplicate backup path"
        );
        ensure!(
            !fs::symlink_metadata(root.join(&e.file))?
                .file_type()
                .is_symlink(),
            "Backup symlinks are not allowed"
        );
        total = total
            .checked_add(e.bytes)
            .ok_or_else(|| anyhow::anyhow!("Backup size overflow"))?;
        ensure!(
            total <= 2 * 1024 * 1024 * 1024,
            "Backup exceeds supported size"
        );
        ensure!(
            fs::metadata(root.join(&e.file))?.len() == e.bytes,
            "Backup size mismatch"
        );
        let bytes = fs::read(root.join(&e.file))?;
        ensure!(
            sha256(&bytes) == e.sha256,
            "Backup hash mismatch: {}",
            e.file
        );
    }
    Ok(manifest)
}
pub fn restore(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !destination.exists(),
        "Restore destination must be new; existing games will not be overwritten"
    );
    let manifest = validate(source)?;
    let stage = destination.with_extension(format!("staging-{}", uuid::Uuid::now_v7()));
    fs::create_dir_all(stage.join("evidence"))?;
    let result = (|| -> Result<()> {
        for e in &manifest.files {
            fs::copy(source.join(&e.file), stage.join(&e.file))?;
        }
        {
            let store = Store::open(stage.join("games.sqlite"))?;
            for game in store.games()? {
                store.replay_journal(game.id)?;
            }
        }
        fs::rename(&stage, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_restores_and_refuses_corruption_or_overwrite() {
        let root = std::env::temp_dir().join(format!("chess-backup-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&root).unwrap();
        let mut store = Store::open(root.join("games.sqlite")).unwrap();
        let id = contracts::GameId::new();
        store
            .create_game(id, chess_core::ChessGame::standard().initial_fen(), 1)
            .unwrap();
        store
            .set_metadata(id, "Alice", "Bob", "*", "paused", 2)
            .unwrap();
        let dest = root.join("backup");
        create(&store, &root, &dest).unwrap();
        restore(&dest, &root.join("restored")).unwrap();
        assert_eq!(
            Store::open(root.join("restored/games.sqlite"))
                .unwrap()
                .metadata(id)
                .unwrap()
                .white,
            "Alice"
        );
        assert!(restore(&dest, &root.join("restored")).is_err());
        fs::write(dest.join("games.sqlite"), b"bad").unwrap();
        assert!(restore(&dest, &root.join("invalid")).is_err());
        assert!(!root.join("invalid").exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

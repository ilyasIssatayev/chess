use crate::native::Reading;
use anyhow::{Result, ensure};
use contracts::GameId;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use storage::{EvidenceRecord, Store};
use vision_inference::runtime::sha256;

pub const LIMIT: u64 = 512 * 1024 * 1024;
pub struct EvidenceStore {
    pub root: PathBuf,
    pub lost: u64,
}
impl EvidenceStore {
    pub fn open(data: &Path) -> Result<Self> {
        let root = data.join("evidence");
        fs::create_dir_all(&root)?;
        for item in fs::read_dir(&root)? {
            let path = item?.path();
            if path.extension().is_some_and(|x| x == "partial") {
                fs::remove_file(path)?;
            }
        }
        Ok(Self { root, lost: 0 })
    }
    pub fn used(&self) -> Result<u64> {
        Ok(fs::read_dir(&self.root)?
            .map(|e| e.and_then(|e| e.metadata()).map(|m| m.len()))
            .collect::<std::io::Result<Vec<_>>>()?
            .iter()
            .sum())
    }
    pub fn save(
        &mut self,
        store: &Store,
        game_id: GameId,
        reading: &Reading,
        kind: &str,
    ) -> Result<Option<String>> {
        if self.used()? + reading.jpeg.len() as u64 > LIMIT {
            self.lost += 1;
            store.activity(game_id,"evidence_loss", &serde_json::json!({"capture_time_us":reading.observation.capture_time,"reason":"retention_limit"}),super::server::utc_us())?;
            return Ok(None);
        }
        let id = uuid::Uuid::now_v7().to_string();
        let tmp = self.root.join(format!("{id}.partial"));
        let path = self.root.join(format!("{id}.jpg"));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(&reading.jpeg)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &path)?;
        // A crash before this record leaves an orphan, never a false manifest claim.
        let result = store.add_evidence(&EvidenceRecord {
            id: id.clone(),
            game_id,
            session_id: reading.observation.session_id.0.to_string(),
            capture_time_us: reading.observation.capture_time.get(),
            sha256: sha256(&reading.jpeg),
            bytes: reading.jpeg.len() as u64,
            model_version: reading.observation.model_version.clone(),
            calibration_version: reading.observation.calibration_version.clone(),
            kind: kind.into(),
        });
        if let Err(e) = result {
            let _ = fs::remove_file(path);
            return Err(e.into());
        }
        Ok(Some(id))
    }
    pub fn read(&self, record: &EvidenceRecord) -> Result<Vec<u8>> {
        ensure!(
            uuid::Uuid::parse_str(&record.id).is_ok(),
            "Invalid evidence ID"
        );
        let bytes = fs::read(self.root.join(format!("{}.jpg", record.id)))?;
        ensure!(
            bytes.len() as u64 == record.bytes && sha256(&bytes) == record.sha256,
            "Evidence is missing or changed"
        );
        Ok(bytes)
    }
}

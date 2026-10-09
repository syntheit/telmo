//! The real history, plus putting items back on the system clipboard.

use super::{
    Source,
    copy::{self, Payload},
};
use crate::model::{Kind, Snapshot};
use crate::store::Store;
use image::DynamicImage;
use std::time::SystemTime;

pub struct Real(pub Store);

impl Source for Real {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let config = self.0.config();
        Ok(Snapshot {
            entries: self.0.entries()?,
            max_items: config.max_items,
            max_days: config.max_days,
        })
    }

    fn version(&self) -> Option<SystemTime> {
        self.0.modified()
    }

    fn pin(&self, id: &str, pinned: bool) -> Result<(), String> {
        self.0.set_pinned(id, pinned)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.0.delete(id)
    }

    fn clear(&self) -> Result<(), String> {
        self.0.clear()
    }

    fn text(&self, id: &str) -> Result<String, String> {
        self.0.text(id)
    }

    fn image(&self, id: &str) -> Result<DynamicImage, String> {
        let entry = self.find(id)?;
        let path = self.0.image_path(&entry);
        image::open(&path).map_err(|e| format!("Can't open the picture ({}): {e}.", path.display()))
    }

    fn copy(&self, id: &str, text: Option<&str>) -> Result<Option<String>, String> {
        let entry = self.find(id)?;
        let payload = match (text, entry.kind) {
            (Some(text), _) => Payload::Text(text.to_string()),
            (None, Kind::Image) => Payload::Image(self.0.image_path(&entry)),
            (None, Kind::File) => Payload::Files(entry.files.clone()),
            (None, _) => Payload::Text(self.0.text(id)?),
        };
        copy::put(&payload)?;
        Ok(text.map(|t| format!("Copied {t}")))
    }
}

impl Real {
    fn find(&self, id: &str) -> Result<crate::model::Entry, String> {
        self.0
            .entries()?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| "That item is no longer in the history.".to_string())
    }
}

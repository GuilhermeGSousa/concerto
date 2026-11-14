use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_lock::OnceCell;

use super::{utils::try_read_relative, AssetId, ContentAssetRoot};

#[derive(Default)]
pub(crate) struct SharedRead {
    outcome: OnceCell<Result<Arc<Vec<u8>>, String>>,
}

impl SharedRead {
    pub(crate) async fn bytes(
        &self,
        root: &ContentAssetRoot,
        address: &str,
    ) -> anyhow::Result<Arc<Vec<u8>>> {
        let outcome = self
            .outcome
            .get_or_init(|| async {
                match try_read_relative(root, address).await {
                    Ok(Some(bytes)) => Ok(Arc::new(bytes)),
                    Ok(None) => Err(format!("no content asset at '{address}'")),
                    Err(error) => Err(format!("{error:#}")),
                }
            })
            .await;
        outcome
            .clone()
            .map_err(|message| anyhow::anyhow!("{message}"))
    }
}

struct FileRead {
    pending: usize,
    read: Arc<SharedRead>,
}

#[derive(Default)]
pub(crate) struct FileReads {
    files: Mutex<HashMap<AssetId, FileRead>>,
}

impl FileReads {
    pub(crate) fn begin(self: &Arc<Self>, file: AssetId) -> PendingRead {
        let mut files = self.files.lock().unwrap();
        let held = files.entry(file).or_insert_with(|| FileRead {
            pending: 0,
            read: Arc::default(),
        });
        held.pending += 1;
        PendingRead {
            reads: Arc::clone(self),
            file,
            read: Arc::clone(&held.read),
        }
    }

    pub(crate) fn clear(&self) {
        self.files.lock().unwrap().clear();
    }
}

pub(crate) struct PendingRead {
    reads: Arc<FileReads>,
    file: AssetId,
    read: Arc<SharedRead>,
}

impl PendingRead {
    pub(crate) fn shared(&self) -> Arc<SharedRead> {
        Arc::clone(&self.read)
    }
}

impl Drop for PendingRead {
    fn drop(&mut self) {
        let mut files = self.reads.files.lock().unwrap();
        let Some(file) = files.get_mut(&self.file) else {
            return;
        };
        if !Arc::ptr_eq(&file.read, &self.read) {
            return;
        }
        file.pending -= 1;
        if file.pending == 0 {
            files.remove(&self.file);
        }
    }
}

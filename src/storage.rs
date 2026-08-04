use atomic_write_file::AtomicWriteFile;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderState {
    pub current_index: usize,
    pub providers: Vec<Provider>,
}

impl ProviderState {
    pub fn default_state() -> Self {
        Self {
            current_index: 0,
            providers: vec![Provider {
                name: "本地演示 API".to_string(),
                base_url: String::new(),
                api_key: String::new(),
                model: String::new(),
                models: Vec::new(),
            }],
        }
    }

    fn normalize(&mut self) {
        if self.providers.is_empty() {
            *self = Self::default_state();
        }
        self.current_index = self
            .current_index
            .min(self.providers.len().saturating_sub(1));
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: String,
    pub created_at: u64,
    pub prompt: String,
    pub provider: String,
    pub model: String,
    pub resolution: String,
    pub image_format: String,
    pub quantity: u32,
    pub image_file: String,
}

#[derive(Clone, Debug)]
pub struct ImagePayload {
    pub bytes: Vec<u8>,
    pub extension: String,
}

#[derive(Clone, Debug)]
pub struct GenerationMetadata {
    pub prompt: String,
    pub provider: String,
    pub model: String,
    pub resolution: String,
    pub image_format: String,
    pub quantity: u32,
}

#[derive(Clone, Debug)]
pub struct AppPaths {
    #[allow(dead_code)]
    pub root: PathBuf,
    #[allow(dead_code)]
    pub data: PathBuf,
    pub history: PathBuf,
    pub providers: PathBuf,
    pub images: PathBuf,
    pub lock: PathBuf,
}

impl AppPaths {
    fn from_root(root: PathBuf) -> Self {
        let data = root.join("data");
        Self {
            root,
            history: data.join("history.json"),
            providers: data.join("providers.json"),
            images: data.join("images"),
            lock: data.join(".storage.lock"),
            data,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Storage {
    paths: AppPaths,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    version: u32,
    records: Vec<HistoryRecord>,
}

impl Storage {
    pub fn from_executable() -> Result<Self, String> {
        let executable =
            std::env::current_exe().map_err(|error| format!("获取应用路径失败：{error}"))?;
        let root = executable
            .parent()
            .ok_or_else(|| "应用路径没有父目录".to_string())?
            .to_path_buf();
        Ok(Self::from_root(root))
    }

    fn from_root(root: PathBuf) -> Self {
        Self {
            paths: AppPaths::from_root(root),
        }
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.paths.images)
            .map_err(|error| format!("创建应用数据目录失败：{error}"))
    }

    fn with_lock<T>(
        &self,
        operation: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.ensure_dirs()?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.paths.lock)
            .map_err(|error| format!("打开应用数据锁失败：{error}"))?;
        lock.lock_exclusive()
            .map_err(|error| format!("锁定应用数据失败：{error}"))?;
        let result = operation(self);
        let unlock_result = lock
            .unlock()
            .map_err(|error| format!("释放应用数据锁失败：{error}"));
        match (result, unlock_result) {
            (Err(error), _) => Err(error),
            (Ok(_value), Err(error)) => Err(error),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    fn read_history_unlocked(&self) -> Result<Vec<HistoryRecord>, String> {
        if !self.paths.history.exists() {
            return Ok(Vec::new());
        }
        let bytes =
            fs::read(&self.paths.history).map_err(|error| format!("读取历史记录失败：{error}"))?;
        let file = serde_json::from_slice::<HistoryFile>(&bytes)
            .map_err(|error| format!("历史记录格式无效：{error}"))?;
        Ok(file.records)
    }

    fn write_history_unlocked(&self, records: &[HistoryRecord]) -> Result<(), String> {
        let file = HistoryFile {
            version: 1,
            records: records.to_vec(),
        };
        write_json_atomic(&self.paths.history, &file, "历史记录")
    }

    pub fn load_history(&self) -> Result<Vec<HistoryRecord>, String> {
        self.with_lock(|storage| storage.read_history_unlocked())
    }

    pub fn load_history_reconciled(&self) -> Result<Vec<HistoryRecord>, String> {
        self.with_lock(|storage| {
            let mut records = storage.read_history_unlocked()?;
            let original_len = records.len();
            records.retain(|record| {
                storage
                    .image_path(&record.image_file)
                    .and_then(|path| fs::symlink_metadata(path).map_err(|error| error.to_string()))
                    .map(|metadata| metadata.file_type().is_file())
                    .unwrap_or(false)
            });
            if records.len() != original_len {
                storage.write_history_unlocked(&records)?;
            }
            let referenced = records
                .iter()
                .map(|record| record.image_file.as_str())
                .collect::<HashSet<_>>();
            if let Ok(entries) = fs::read_dir(&storage.paths.images) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    if is_generated_image_name(name)
                        && !referenced.contains(name)
                        && fs::symlink_metadata(&path)
                            .map(|metadata| metadata.file_type().is_file())
                            .unwrap_or(false)
                    {
                        let _ = fs::remove_file(path);
                    }
                }
            }
            Ok(records)
        })
    }

    pub fn image_path(&self, image_file: &str) -> Result<PathBuf, String> {
        validate_image_file_name(image_file)?;
        Ok(self.paths.images.join(image_file))
    }

    pub fn store_images(
        &self,
        images: &[ImagePayload],
        metadata: &GenerationMetadata,
    ) -> Result<Vec<HistoryRecord>, String> {
        if images.is_empty() {
            return Err("没有可保存的图片".to_string());
        }
        self.with_lock(|storage| {
            let mut paths = Vec::with_capacity(images.len());
            let mut records = Vec::with_capacity(images.len());
            for image in images {
                if image.bytes.is_empty() {
                    remove_files(&paths);
                    return Err("图片内容为空，无法保存".to_string());
                }
                let id = unique_id();
                let extension = normalize_extension(&image.extension);
                let image_file = format!("imggen-{id}.{extension}");
                let path = storage.image_path(&image_file)?;
                if let Err(error) = write_image_atomically(&path, &image.bytes, &id) {
                    remove_files(&paths);
                    return Err(error);
                }
                paths.push(path);
                records.push(HistoryRecord {
                    id,
                    created_at: unix_seconds(),
                    prompt: metadata.prompt.clone(),
                    provider: metadata.provider.clone(),
                    model: metadata.model.clone(),
                    resolution: metadata.resolution.clone(),
                    image_format: metadata.image_format.clone(),
                    quantity: metadata.quantity,
                    image_file,
                });
            }
            let mut all_records = match storage.read_history_unlocked() {
                Ok(records) => records,
                Err(error) => {
                    remove_files(&paths);
                    return Err(error);
                }
            };
            for record in records.iter().rev() {
                all_records.insert(0, record.clone());
            }
            if let Err(error) = storage.write_history_unlocked(&all_records) {
                remove_files(&paths);
                return Err(error);
            }
            Ok(records)
        })
    }

    pub fn delete_history(&self, id: &str) -> Result<Option<HistoryRecord>, String> {
        if id.trim().is_empty() {
            return Err("历史记录 ID 不能为空".to_string());
        }
        self.with_lock(|storage| {
            let mut records = storage.read_history_unlocked()?;
            let Some(index) = records.iter().position(|record| record.id == id) else {
                return Ok(None);
            };
            let record = records.remove(index);
            let image_path = storage.image_path(&record.image_file)?;
            let tombstone = storage.paths.images.join(format!(
                ".{}.deleting-{}",
                record.image_file,
                unique_id()
            ));
            let moved = if let Ok(metadata) = fs::symlink_metadata(&image_path) {
                if !metadata.file_type().is_file() {
                    return Err("历史图片不是普通文件，拒绝删除".to_string());
                }
                fs::rename(&image_path, &tombstone)
                    .map_err(|error| format!("暂存历史图片失败：{error}"))?;
                true
            } else {
                false
            };
            if let Err(error) = storage.write_history_unlocked(&records) {
                if moved {
                    let _ = fs::rename(&tombstone, &image_path);
                }
                return Err(error);
            }
            if moved && let Err(error) = fs::remove_file(&tombstone) {
                records.insert(index, record.clone());
                let history_restore = storage.write_history_unlocked(&records).err();
                let image_restore = fs::rename(&tombstone, &image_path).err();
                let detail = match (history_restore, image_restore) {
                    (None, None) => "，已恢复历史记录和图片".to_string(),
                    (history_error, image_error) => format!(
                        "，恢复结果：历史记录={}，图片={}",
                        restore_result(history_error),
                        restore_result(image_error)
                    ),
                };
                return Err(format!("删除历史图片失败：{error}{detail}"));
            }
            Ok(Some(record))
        })
    }

    fn read_provider_state_unlocked(&self) -> Result<ProviderState, String> {
        if !self.paths.providers.exists() {
            return Ok(ProviderState::default_state());
        }
        let bytes = fs::read(&self.paths.providers)
            .map_err(|error| format!("读取 API 提供商配置失败：{error}"))?;
        let mut state = serde_json::from_slice::<ProviderState>(&bytes)
            .map_err(|error| format!("API 提供商配置格式无效：{error}"))?;
        state.normalize();
        Ok(state)
    }

    fn write_provider_state_unlocked(&self, state: &ProviderState) -> Result<(), String> {
        write_json_atomic(&self.paths.providers, state, "API 提供商配置")
    }

    pub fn load_provider_state(&self) -> Result<ProviderState, String> {
        self.with_lock(|storage| storage.read_provider_state_unlocked())
    }

    pub fn save_provider_state(&self, mut state: ProviderState) -> Result<(), String> {
        state.normalize();
        self.with_lock(|storage| storage.write_provider_state_unlocked(&state))
    }

    pub fn update_provider_state<T>(
        &self,
        update: impl FnOnce(&mut ProviderState) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_lock(|storage| {
            let mut state = storage.read_provider_state_unlocked()?;
            let value = update(&mut state)?;
            state.normalize();
            storage.write_provider_state_unlocked(&state)?;
            Ok(value)
        })
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T, label: &str) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("序列化{label}失败：{error}"))?;
    let mut file = AtomicWriteFile::options()
        .open(path)
        .map_err(|error| format!("打开{label}临时文件失败：{error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("写入{label}失败：{error}"))?;
    file.commit()
        .map_err(|error| format!("提交{label}失败：{error}"))
}

fn write_image_atomically(path: &Path, bytes: &[u8], id: &str) -> Result<(), String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "图片文件名无效".to_string())?;
    let temporary = path
        .parent()
        .ok_or_else(|| "图片目录不存在".to_string())?
        .join(format!(".{file_name}.tmp-{id}"));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("创建图片临时文件失败：{error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("写入图片失败：{error}"))?;
        file.sync_all()
            .map_err(|error| format!("刷新图片失败：{error}"))?;
        fs::rename(&temporary, path).map_err(|error| format!("提交图片失败：{error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn restore_result<T>(error: Option<T>) -> &'static str {
    if error.is_none() { "成功" } else { "失败" }
}

fn remove_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn normalize_extension(extension: &str) -> &'static str {
    match extension.trim().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "jpg",
        "webp" => "webp",
        _ => "png",
    }
}

fn validate_image_file_name(image_file: &str) -> Result<(), String> {
    let path = Path::new(image_file);
    let valid = !image_file.is_empty()
        && !image_file.contains('/')
        && !image_file.contains('\\')
        && path.file_name().and_then(|name| name.to_str()) == Some(image_file)
        && image_file.starts_with("imggen-")
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("png" | "jpg" | "webp")
        );
    if valid {
        Ok(())
    } else {
        Err("历史图片文件名无效".to_string())
    }
}

fn is_generated_image_name(name: &str) -> bool {
    validate_image_file_name(name).is_ok()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unique_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let counter = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{millis}-{}-{counter}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::{GenerationMetadata, ImagePayload, Storage};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_storage() -> Storage {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("imggen-storage-test-{stamp}"));
        let _ = fs::remove_dir_all(&root);
        Storage::from_root(root)
    }

    fn metadata() -> GenerationMetadata {
        GenerationMetadata {
            prompt: "测试图片".to_string(),
            provider: "测试 API".to_string(),
            model: "test-model".to_string(),
            resolution: "1024 × 1024".to_string(),
            image_format: "PNG".to_string(),
            quantity: 2,
        }
    }

    #[test]
    fn stores_and_deletes_each_image_with_history() {
        let storage = test_storage();
        let records = storage
            .store_images(
                &[
                    ImagePayload {
                        bytes: b"one".to_vec(),
                        extension: "png".to_string(),
                    },
                    ImagePayload {
                        bytes: b"two".to_vec(),
                        extension: "png".to_string(),
                    },
                ],
                &metadata(),
            )
            .unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(storage.load_history().unwrap().len(), 2);
        for record in &records {
            assert!(storage.image_path(&record.image_file).unwrap().exists());
        }
        let deleted = storage.delete_history(&records[0].id).unwrap().unwrap();
        assert!(!storage.image_path(&deleted.image_file).unwrap().exists());
        assert_eq!(storage.load_history().unwrap().len(), 1);
        assert!(storage.delete_history("../../outside").unwrap().is_none());
        let _ = fs::remove_dir_all(storage.paths.root.clone());
    }

    #[test]
    fn rejects_malicious_history_file_name() {
        let storage = test_storage();
        let record = super::HistoryRecord {
            id: "malicious".to_string(),
            created_at: 0,
            prompt: "bad".to_string(),
            provider: "bad".to_string(),
            model: "bad".to_string(),
            resolution: "1024 × 1024".to_string(),
            image_format: "PNG".to_string(),
            quantity: 1,
            image_file: "../outside.png".to_string(),
        };
        let file = serde_json::json!({ "version": 1, "records": [record] });
        storage.ensure_dirs().unwrap();
        fs::write(&storage.paths.history, serde_json::to_vec(&file).unwrap()).unwrap();
        assert!(storage.load_history().is_ok());
        assert!(storage.delete_history("malicious").is_err());
        let _ = fs::remove_dir_all(storage.paths.root.clone());
    }

    #[test]
    fn reloads_history_and_removes_missing_images() {
        let storage = test_storage();
        let records = storage
            .store_images(
                &[ImagePayload {
                    bytes: b"one".to_vec(),
                    extension: "jpeg".to_string(),
                }],
                &metadata(),
            )
            .unwrap();
        fs::remove_file(storage.image_path(&records[0].image_file).unwrap()).unwrap();
        assert!(storage.load_history_reconciled().unwrap().is_empty());
        let _ = fs::remove_dir_all(storage.paths.root.clone());
    }

    #[test]
    fn provider_state_round_trips_without_history_data() {
        let storage = test_storage();
        let mut state = storage.load_provider_state().unwrap();
        state.providers[0].api_key = "secret".to_string();
        state.providers[0].model = "model".to_string();
        storage.save_provider_state(state.clone()).unwrap();
        let loaded = storage.load_provider_state().unwrap();
        assert_eq!(loaded.providers[0].api_key, "secret");
        assert!(!serde_json::to_string(&loaded).unwrap().is_empty());
        assert!(storage.load_history().unwrap().is_empty());
        let _ = fs::remove_dir_all(storage.paths.root.clone());
    }
}

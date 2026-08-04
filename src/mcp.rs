use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use tokio::task;

use crate::storage::{HistoryRecord, Provider, ProviderState, Storage};
use crate::{generate_and_store, request_models};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GenerateImageRequest {
    #[schemars(description = "图片提示词")]
    pub prompt: String,
    #[schemars(description = "输出分辨率，例如 1024 × 1024")]
    pub resolution: Option<String>,
    #[schemars(description = "输出格式：PNG、JPEG 或 WEBP")]
    pub image_format: Option<String>,
    #[schemars(description = "生成数量，范围 1 到 10")]
    pub quantity: Option<u32>,
    #[schemars(description = "可选的已配置 API 提供商名称")]
    pub provider: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteHistoryRequest {
    #[schemars(description = "历史记录 ID")]
    pub id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SaveProviderRequest {
    #[schemars(description = "API 提供商名称")]
    pub name: String,
    #[schemars(description = "OpenAI-compatible Base URL")]
    pub base_url: String,
    #[schemars(description = "API Key；省略时保留已有密钥")]
    pub api_key: Option<String>,
    #[schemars(description = "模型名称")]
    pub model: Option<String>,
    #[schemars(description = "可选模型列表")]
    pub models: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProviderNameRequest {
    #[schemars(description = "API 提供商名称")]
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
struct HistoryOutput {
    id: String,
    created_at: u64,
    prompt: String,
    provider: String,
    model: String,
    resolution: String,
    image_format: String,
    quantity: u32,
    image_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderOutput {
    name: String,
    base_url: String,
    model: String,
    models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImgGenMcp {
    storage: Storage,
}

impl ImgGenMcp {
    fn new(storage: Storage) -> Self {
        Self { storage }
    }

    fn history_output(&self, record: HistoryRecord) -> Result<HistoryOutput, String> {
        let image_path = self.storage.image_path(&record.image_file)?;
        Ok(HistoryOutput {
            id: record.id,
            created_at: record.created_at,
            prompt: record.prompt,
            provider: record.provider,
            model: record.model,
            resolution: record.resolution,
            image_format: record.image_format,
            quantity: record.quantity,
            image_path: image_path.to_string_lossy().into_owned(),
        })
    }

    fn provider_output(provider: &Provider) -> ProviderOutput {
        ProviderOutput {
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            models: provider.models.clone(),
        }
    }
}

#[tool_router]
impl ImgGenMcp {
    #[tool(description = "生成图片，并将每张图片保存为历史记录")]
    async fn generate_image(
        &self,
        Parameters(request): Parameters<GenerateImageRequest>,
    ) -> Result<String, String> {
        if request.prompt.trim().is_empty() {
            return Err("prompt 不能为空".to_string());
        }
        let storage = self.storage.clone();
        let state = storage.load_provider_state()?;
        let provider = select_provider(&state, request.provider.as_deref())?.clone();
        let prompt = request.prompt;
        let resolution = request
            .resolution
            .unwrap_or_else(|| "1024 × 1024".to_string());
        let image_format = request.image_format.unwrap_or_else(|| "PNG".to_string());
        let quantity = request.quantity.unwrap_or(1).clamp(1, 10);
        let records = task::spawn_blocking(move || {
            generate_and_store(
                &storage,
                &provider,
                &prompt,
                &resolution,
                &image_format,
                quantity,
            )
        })
        .await
        .map_err(|error| format!("生成任务失败：{error}"))??;
        let output = records
            .into_iter()
            .map(|record| self.history_output(record))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_string(&output).map_err(|error| format!("生成结果序列化失败：{error}"))
    }

    #[tool(description = "查询已保存的图片历史记录")]
    async fn list_history(&self) -> Result<String, String> {
        let storage = self.storage.clone();
        let records = task::spawn_blocking(move || storage.load_history_reconciled())
            .await
            .map_err(|error| format!("读取历史任务失败：{error}"))??;
        let output = records
            .into_iter()
            .map(|record| self.history_output(record))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_string(&output).map_err(|error| format!("历史结果序列化失败：{error}"))
    }

    #[tool(description = "删除图片历史记录及其关联图片")]
    async fn delete_history(
        &self,
        Parameters(request): Parameters<DeleteHistoryRequest>,
    ) -> Result<String, String> {
        let storage = self.storage.clone();
        let id = request.id;
        let id_for_delete = id.clone();
        let deleted = task::spawn_blocking(move || storage.delete_history(&id_for_delete))
            .await
            .map_err(|error| format!("删除历史任务失败：{error}"))??;
        serde_json::to_string(&serde_json::json!({ "deleted": deleted.is_some(), "id": id }))
            .map_err(|error| format!("删除结果序列化失败：{error}"))
    }

    #[tool(description = "查询已配置的 API 提供商，不返回 API Key")]
    async fn list_providers(&self) -> Result<String, String> {
        let state = self.storage.load_provider_state()?;
        let providers = state
            .providers
            .iter()
            .map(Self::provider_output)
            .collect::<Vec<_>>();
        serde_json::to_string(&serde_json::json!({
            "current_index": state.current_index,
            "providers": providers,
        }))
        .map_err(|error| format!("供应商结果序列化失败：{error}"))
    }

    #[tool(description = "保存或更新 API 提供商配置")]
    async fn save_provider(
        &self,
        Parameters(request): Parameters<SaveProviderRequest>,
    ) -> Result<String, String> {
        if request.name.trim().is_empty() {
            return Err("供应商名称不能为空".to_string());
        }
        if request.base_url.trim().is_empty() {
            return Err("Base URL 不能为空".to_string());
        }
        let provider = self.storage.update_provider_state(|state| {
            let existing = state
                .providers
                .iter()
                .position(|provider| provider.name == request.name);
            if let Some(index) = existing {
                let provider = &mut state.providers[index];
                provider.base_url = request.base_url.clone();
                if let Some(api_key) = request.api_key.clone() {
                    provider.api_key = api_key;
                }
                if let Some(model) = request.model.clone() {
                    provider.model = model;
                }
                if let Some(models) = request.models.clone() {
                    provider.models = models;
                }
                Ok(provider.clone())
            } else {
                let provider = Provider {
                    name: request.name.clone(),
                    base_url: request.base_url.clone(),
                    api_key: request.api_key.clone().unwrap_or_default(),
                    model: request.model.clone().unwrap_or_default(),
                    models: request.models.clone().unwrap_or_default(),
                };
                state.providers.push(provider.clone());
                Ok(provider)
            }
        })?;
        serde_json::to_string(&Self::provider_output(&provider))
            .map_err(|error| format!("供应商结果序列化失败：{error}"))
    }

    #[tool(description = "删除 API 提供商")]
    async fn delete_provider(
        &self,
        Parameters(request): Parameters<ProviderNameRequest>,
    ) -> Result<String, String> {
        let deleted = self.storage.update_provider_state(|state| {
            if state.providers.len() <= 1 {
                return Err("至少保留一个 API 提供商".to_string());
            }
            let Some(index) = state
                .providers
                .iter()
                .position(|provider| provider.name == request.name)
            else {
                return Ok(false);
            };
            state.providers.remove(index);
            if state.current_index > index {
                state.current_index -= 1;
            } else if state.current_index >= state.providers.len() {
                state.current_index = state.providers.len() - 1;
            }
            Ok(true)
        })?;
        serde_json::to_string(&serde_json::json!({ "deleted": deleted, "name": request.name }))
            .map_err(|error| format!("删除供应商结果序列化失败：{error}"))
    }

    #[tool(description = "切换当前 API 提供商")]
    async fn select_provider(
        &self,
        Parameters(request): Parameters<ProviderNameRequest>,
    ) -> Result<String, String> {
        let index = self.storage.update_provider_state(|state| {
            let index = state
                .providers
                .iter()
                .position(|provider| provider.name == request.name)
                .ok_or_else(|| "没有找到指定 API 提供商".to_string())?;
            state.current_index = index;
            Ok(index)
        })?;
        serde_json::to_string(&serde_json::json!({ "name": request.name, "current_index": index }))
            .map_err(|error| format!("切换供应商结果序列化失败：{error}"))
    }

    #[tool(description = "从当前 API 提供商获取模型列表并保存")]
    async fn fetch_provider_models(
        &self,
        Parameters(request): Parameters<ProviderNameRequest>,
    ) -> Result<String, String> {
        let state = self.storage.load_provider_state()?;
        let provider = select_provider(&state, Some(&request.name))?.clone();
        let models = task::spawn_blocking(move || request_models(&provider))
            .await
            .map_err(|error| format!("获取模型任务失败：{error}"))??;
        let models_for_output = models.clone();
        self.storage.update_provider_state(|state| {
            let provider = state
                .providers
                .iter_mut()
                .find(|provider| provider.name == request.name)
                .ok_or_else(|| "没有找到指定 API 提供商".to_string())?;
            provider.models = models;
            if provider.model.is_empty()
                && let Some(first) = provider.models.first()
            {
                provider.model = first.clone();
            }
            Ok(())
        })?;
        serde_json::to_string(
            &serde_json::json!({ "name": request.name, "models": models_for_output }),
        )
        .map_err(|error| format!("模型结果序列化失败：{error}"))
    }
}

#[tool_handler]
impl ServerHandler for ImgGenMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("ImgGen 图片生成、历史和 API 提供商管理服务")
    }
}

fn select_provider<'a>(
    state: &'a ProviderState,
    name: Option<&str>,
) -> Result<&'a Provider, String> {
    if let Some(name) = name {
        state
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .ok_or_else(|| "没有找到指定 API 提供商".to_string())
    } else {
        state
            .providers
            .get(state.current_index)
            .ok_or_else(|| "没有可用的 API 提供商".to_string())
    }
}

pub fn run(storage: Storage) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        storage.ensure_dirs().map_err(std::io::Error::other)?;
        let server = ImgGenMcp::new(storage);
        let (stdin, stdout) = rmcp::transport::io::stdio();
        server.serve((stdin, stdout)).await?.waiting().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[cfg(test)]
mod tests {
    use super::select_provider;
    use crate::storage::ProviderState;

    #[test]
    fn selects_current_or_named_provider() {
        let state = ProviderState::default_state();
        assert_eq!(select_provider(&state, None).unwrap().name, "本地演示 API");
        assert!(select_provider(&state, Some("missing")).is_err());
    }

    #[test]
    fn provider_output_does_not_include_api_key() {
        let provider = crate::storage::Provider {
            name: "test".to_string(),
            base_url: "https://example.com".to_string(),
            api_key: "secret".to_string(),
            model: "model".to_string(),
            models: vec!["model".to_string()],
        };
        let output = serde_json::to_string(&super::ImgGenMcp::provider_output(&provider)).unwrap();
        assert!(!output.contains("secret"));
        assert!(!output.contains("api_key"));
    }
}

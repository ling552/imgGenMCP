mod mcp;
mod storage;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use slint::{Image, ModelRc, SharedString, VecModel};
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use storage::{GenerationMetadata, HistoryRecord, ImagePayload, Provider, ProviderState, Storage};

slint::include_modules!();

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelItem>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: Option<serde_json::Value>,
    message: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageItem {
    b64_json: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImagesResponse {
    data: Vec<ImageItem>,
}

fn endpoint(base_url: &str, path: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if base_url.ends_with(path) {
        base_url.to_string()
    } else {
        format!("{base_url}/{path}")
    }
}

fn body_summary(body: &str) -> String {
    body.trim().chars().take(240).collect()
}

fn error_detail(body: &str) -> String {
    serde_json::from_str::<ApiErrorResponse>(body)
        .ok()
        .and_then(|payload| {
            let nested = payload.error.and_then(|error| {
                error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| error.as_str().map(str::to_owned))
            });
            nested.or(payload.message).or(payload.detail)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| body_summary(body))
}

fn format_http_error(action: &str, status: &str, retry_after: &str, body: &str) -> String {
    let detail = error_detail(body);
    if detail.is_empty() {
        format!("{action}失败：HTTP {status}{retry_after}")
    } else {
        format!("{action}失败：HTTP {status}{retry_after}：{detail}")
    }
}

fn http_error(action: &str, response: reqwest::blocking::Response) -> String {
    let status = response.status().to_string();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("，Retry-After: {value}"))
        .unwrap_or_default();
    let body = response.text().unwrap_or_default();
    format_http_error(action, &status, &retry_after, &body)
}

fn request_error(action: &str, error: reqwest::Error, url: &str) -> String {
    let reason = if error.is_timeout() {
        "请求超时"
    } else if error.is_connect() {
        "连接服务器失败"
    } else {
        "网络请求失败"
    };
    let source = error
        .source()
        .map(|cause| format!("；原因：{cause}"))
        .unwrap_or_default();
    format!("{action}失败：{reason}（{url}）：{error}{source}")
}

pub(crate) fn request_models(provider: &Provider) -> Result<Vec<String>, String> {
    if provider.base_url.trim().is_empty() {
        return Err("API 提供商未配置 Base URL，无法获取模型".to_string());
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("创建模型请求客户端失败：{error}"))?;
    let models_url = endpoint(&provider.base_url, "models");
    let mut request = client.get(&models_url);
    if !provider.api_key.trim().is_empty() {
        request = request.bearer_auth(&provider.api_key);
    }

    let response = request
        .send()
        .map_err(|error| request_error("获取模型", error, &models_url))?;
    if !response.status().is_success() {
        return Err(http_error("获取模型", response));
    }
    let body = response
        .text()
        .map_err(|error| format!("读取模型响应失败：{error}"))?;
    let models = serde_json::from_str::<ModelsResponse>(&body)
        .map_err(|_| format!("模型响应格式无效：{}", body_summary(&body)))?
        .data
        .into_iter()
        .map(|model| model.id)
        .filter(|model| !model.trim().is_empty())
        .collect::<Vec<_>>();

    if models.is_empty() {
        Err("供应商没有返回可用模型".to_string())
    } else {
        Ok(models)
    }
}

pub(crate) fn request_image(
    provider: &Provider,
    prompt: &str,
    resolution: &str,
    image_format: &str,
    quantity: u32,
) -> Result<Vec<ImagePayload>, String> {
    if provider.base_url.trim().is_empty() {
        return Err("API 提供商未配置 Base URL".to_string());
    }
    if provider.api_key.trim().is_empty() {
        return Err("API 提供商未配置 API Key".to_string());
    }
    if provider.model.trim().is_empty() {
        return Err("API 提供商未配置模型".to_string());
    }

    let size = match resolution.trim() {
        "1024 × 1792" | "1024x1792" => "1024x1792",
        "1792 × 1024" | "1792x1024" => "1792x1024",
        _ => "1024x1024",
    };
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("创建生图请求客户端失败：{error}"))?;
    let image_url = endpoint(&provider.base_url, "images/generations");
    let response = client
        .post(&image_url)
        .bearer_auth(&provider.api_key)
        .json(&json!({
            "model": provider.model,
            "prompt": prompt,
            "n": quantity.clamp(1, 10),
            "size": size
        }))
        .send()
        .map_err(|error| request_error("生成图片", error, &image_url))?;
    if !response.status().is_success() {
        return Err(http_error("图片接口请求", response));
    }
    let body = response
        .text()
        .map_err(|error| format!("读取图片响应失败：{error}"))?;
    let result = serde_json::from_str::<ImagesResponse>(&body)
        .map_err(|_| format!("图片响应格式无效：{}", body_summary(&body)))?;
    if result.data.is_empty() {
        return Err("图片接口没有返回图片".to_string());
    }

    result
        .data
        .into_iter()
        .map(|item| {
            let bytes = if let Some(encoded) = item.b64_json {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|error| format!("图片数据解码失败：{error}"))?
            } else if let Some(url) = item.url.filter(|url| !url.trim().is_empty()) {
                let image_response = client
                    .get(&url)
                    .send()
                    .map_err(|error| request_error("下载图片", error, &url))?;
                if !image_response.status().is_success() {
                    return Err(http_error("下载图片", image_response));
                }
                let bytes = image_response
                    .bytes()
                    .map_err(|error| format!("读取图片失败：{error}"))?;
                if bytes.is_empty() {
                    return Err("下载图片失败：响应内容为空".to_string());
                }
                bytes.to_vec()
            } else {
                return Err("图片响应缺少 b64_json 或 url".to_string());
            };
            if bytes.is_empty() {
                return Err("图片内容为空".to_string());
            }
            let extension = image_extension(&bytes, image_format);
            Ok(ImagePayload { bytes, extension })
        })
        .collect()
}

fn image_extension(bytes: &[u8], fallback: &str) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png".to_string()
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpg".to_string()
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp".to_string()
    } else {
        match fallback.trim().to_ascii_lowercase().as_str() {
            "jpeg" | "jpg" => "jpg".to_string(),
            "webp" => "webp".to_string(),
            _ => "png".to_string(),
        }
    }
}

pub(crate) fn generate_and_store(
    storage: &Storage,
    provider: &Provider,
    prompt: &str,
    resolution: &str,
    image_format: &str,
    quantity: u32,
) -> Result<Vec<HistoryRecord>, String> {
    let images = request_image(provider, prompt, resolution, image_format, quantity)?;
    storage.store_images(
        &images,
        &GenerationMetadata {
            prompt: prompt.to_string(),
            provider: provider.name.clone(),
            model: provider.model.clone(),
            resolution: resolution.to_string(),
            image_format: image_format.to_string(),
            quantity: quantity.clamp(1, 10),
        },
    )
}

fn provider_names(providers: &[Provider]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        providers
            .iter()
            .map(|provider| provider.name.clone().into())
            .collect::<Vec<SharedString>>(),
    ))
}

fn provider_models(providers: &[Provider], index: usize) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        providers
            .get(index)
            .map(|provider| {
                provider
                    .models
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    ))
}

fn load_provider(app: &App, provider: &Provider, index: usize) {
    app.set_current_provider_index(index as i32);
    app.set_current_provider_name(provider.name.clone().into());
    app.set_current_base_url(provider.base_url.clone().into());
    app.set_current_api_key(provider.api_key.clone().into());
    app.set_current_model(provider.model.clone().into());
    app.set_available_models(provider_models(std::slice::from_ref(provider), 0));
}

fn save_current_provider(app: &App, providers: &Arc<Mutex<Vec<Provider>>>) {
    let index = app.get_current_provider_index() as usize;
    let mut providers = providers.lock().expect("provider state poisoned");
    if let Some(provider) = providers.get_mut(index) {
        provider.name = app.get_current_provider_name().to_string();
        provider.base_url = app.get_current_base_url().to_string();
        provider.api_key = app.get_current_api_key().to_string();
        provider.model = app.get_current_model().to_string();
    }
}

fn persist_provider_state(
    app: &App,
    providers: &Arc<Mutex<Vec<Provider>>>,
    storage: &Storage,
) -> Result<(), String> {
    save_current_provider(app, providers);
    let current_index = app.get_current_provider_index() as usize;
    let providers = providers.lock().expect("provider state poisoned").clone();
    storage.save_provider_state(ProviderState {
        current_index,
        providers,
    })
}

fn history_model(storage: &Storage, records: &[HistoryRecord]) -> ModelRc<HistoryItem> {
    let items = records
        .iter()
        .filter_map(|record| {
            let path = storage.image_path(&record.image_file).ok()?;
            let thumbnail = Image::load_from_path(&path).ok()?;
            let title = record.prompt.chars().take(22).collect::<String>();
            let subtitle = format!("{}/{}", record.provider, record.model);
            Some(HistoryItem {
                id: record.id.clone().into(),
                title: if title.is_empty() {
                    "未命名图片".to_string().into()
                } else {
                    title.into()
                },
                subtitle: subtitle.into(),
                thumbnail,
            })
        })
        .collect::<Vec<_>>();
    ModelRc::new(VecModel::from(items))
}

fn refresh_history(app: &App, storage: &Storage) -> Result<Vec<HistoryRecord>, String> {
    let records = storage.load_history_reconciled()?;
    app.set_history_items(history_model(storage, &records));
    Ok(records)
}

fn select_history_record(app: &App, storage: &Storage, id: &str) -> Result<(), String> {
    let records = storage.load_history()?;
    let record = records
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| "没有找到对应的历史记录".to_string())?;
    let path = storage.image_path(&record.image_file)?;
    let image =
        Image::load_from_path(&path).map_err(|error| format!("加载历史图片失败：{error}"))?;
    app.set_preview_image(image);
    app.set_showing_history_preview(true);
    app.set_selected_history_id(id.to_string().into());
    app.set_status_text("已载入历史图片".into());
    Ok(())
}

fn run_gui(storage: Storage) -> Result<(), Box<dyn Error>> {
    storage.ensure_dirs()?;
    let provider_state = storage.load_provider_state()?;
    storage.save_provider_state(provider_state.clone())?;
    let history = storage.load_history_reconciled()?;
    let app = App::new()?;
    let providers = Arc::new(Mutex::new(provider_state.providers));
    let current_index = provider_state
        .current_index
        .min(providers.lock().expect("provider state poisoned").len() - 1);
    {
        let providers_guard = providers.lock().expect("provider state poisoned");
        app.set_provider_names(provider_names(&providers_guard));
        load_provider(&app, &providers_guard[current_index], current_index);
    }
    app.set_history_items(history_model(&storage, &history));

    {
        let weak = app.as_weak();
        app.on_toggle_sidebar(move || {
            if let Some(app) = weak.upgrade() {
                let collapsed = app.get_sidebar_collapsed();
                app.set_sidebar_collapsed(!collapsed);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_open_settings(move || {
            if let Some(app) = weak.upgrade() {
                app.set_settings_open(true);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_close_settings(move || {
            if let Some(app) = weak.upgrade() {
                app.set_settings_open(false);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_toggle_theme(move || {
            if let Some(app) = weak.upgrade() {
                let dark = app.get_dark_theme();
                app.set_dark_theme(!dark);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_apply_custom_color(move || {
            if let Some(app) = weak.upgrade() {
                app.set_accent_color(app.get_selected_theme_color());
                app.set_status_text("主题色已应用".into());
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_select_provider(move |index| {
            if let Some(app) = weak.upgrade() {
                if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                    app.set_status_text(error.into());
                    return;
                }
                let provider = providers_ref
                    .lock()
                    .expect("provider state poisoned")
                    .get(index as usize)
                    .cloned();
                if let Some(provider) = provider {
                    load_provider(&app, &provider, index as usize);
                    app.set_status_text(format!("切换 API 提供商成功：{}", provider.name).into());
                    if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                        app.set_status_text(error.into());
                    }
                }
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_add_provider(move || {
            if let Some(app) = weak.upgrade() {
                if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                    app.set_status_text(error.into());
                    return;
                }
                let mut providers = providers_ref.lock().expect("provider state poisoned");
                let index = providers.len();
                providers.push(Provider {
                    name: format!("新 API 提供商 {}", index + 1),
                    base_url: String::new(),
                    api_key: String::new(),
                    model: String::new(),
                    models: Vec::new(),
                });
                app.set_provider_names(provider_names(&providers));
                load_provider(&app, &providers[index], index);
                drop(providers);
                match persist_provider_state(&app, &providers_ref, &storage_ref) {
                    Ok(()) => app.set_status_text(
                        format!("新建 API 提供商成功：新 API 提供商 {}", index + 1).into(),
                    ),
                    Err(error) => app.set_status_text(error.into()),
                }
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_copy_provider(move |source_index| {
            if let Some(app) = weak.upgrade() {
                if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                    app.set_status_text(error.into());
                    return;
                }
                let mut providers = providers_ref.lock().expect("provider state poisoned");
                if providers.is_empty() {
                    app.set_status_text("没有可复制的 API 提供商".into());
                    return;
                }
                let source_index = (source_index as usize).min(providers.len() - 1);
                let mut copied = providers[source_index].clone();
                copied.name = format!("{} 副本", copied.name);
                let index = providers.len();
                let name = copied.name.clone();
                providers.push(copied);
                app.set_provider_names(provider_names(&providers));
                load_provider(&app, &providers[index], index);
                drop(providers);
                match persist_provider_state(&app, &providers_ref, &storage_ref) {
                    Ok(()) => app.set_status_text(format!("复制 API 提供商成功：{name}").into()),
                    Err(error) => app.set_status_text(error.into()),
                }
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_delete_provider(move |source_index| {
            if let Some(app) = weak.upgrade() {
                if providers_ref.lock().expect("provider state poisoned").len() <= 1 {
                    app.set_status_text("至少保留一个 API 提供商".into());
                    return;
                }
                if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                    app.set_status_text(error.into());
                    return;
                }
                let mut providers = providers_ref.lock().expect("provider state poisoned");
                let remove_index = (source_index as usize).min(providers.len() - 1);
                providers.remove(remove_index);
                let next_index = remove_index.min(providers.len() - 1);
                app.set_provider_names(provider_names(&providers));
                load_provider(&app, &providers[next_index], next_index);
                drop(providers);
                match persist_provider_state(&app, &providers_ref, &storage_ref) {
                    Ok(()) => app.set_status_text("删除 API 提供商成功".into()),
                    Err(error) => app.set_status_text(error.into()),
                }
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_save_provider(move || {
            if let Some(app) = weak.upgrade() {
                match persist_provider_state(&app, &providers_ref, &storage_ref) {
                    Ok(()) => {
                        let names =
                            provider_names(&providers_ref.lock().expect("provider state poisoned"));
                        app.set_provider_names(names);
                        app.set_status_text(
                            format!("保存 API 提供商成功：{}", app.get_current_provider_name())
                                .into(),
                        );
                    }
                    Err(error) => app.set_status_text(error.into()),
                }
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_fetch_models(move || {
            if let Some(app) = weak.upgrade() {
                if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                    app.set_status_text(error.into());
                    return;
                }
                let index = app.get_current_provider_index() as usize;
                let provider = providers_ref
                    .lock()
                    .expect("provider state poisoned")
                    .get(index)
                    .cloned();
                let Some(provider) = provider else {
                    app.set_status_text("没有找到当前 API 提供商".into());
                    return;
                };
                app.set_loading_models(true);
                app.set_status_text("正在获取模型，请稍候...".into());
                let weak = app.as_weak();
                let providers_for_thread = providers_ref.clone();
                let storage_for_thread = storage_ref.clone();
                thread::spawn(move || {
                    let result = request_models(&provider);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = weak.upgrade() {
                            app.set_loading_models(false);
                            match result {
                                Ok(models) => {
                                    let first = models.first().cloned();
                                    {
                                        let mut providers = providers_for_thread
                                            .lock()
                                            .expect("provider state poisoned");
                                        if let Some(provider) = providers.get_mut(index) {
                                            provider.models = models.clone();
                                            if let Some(first) = &first {
                                                provider.model = first.clone();
                                            }
                                        }
                                    }
                                    if app.get_current_provider_index() as usize == index {
                                        app.set_available_models(ModelRc::new(VecModel::from(
                                            models
                                                .iter()
                                                .cloned()
                                                .map(SharedString::from)
                                                .collect::<Vec<_>>(),
                                        )));
                                        if let Some(first) = first {
                                            app.set_current_model(first.into());
                                        }
                                    }
                                    if let Err(error) = persist_provider_state(
                                        &app,
                                        &providers_for_thread,
                                        &storage_for_thread,
                                    ) {
                                        app.set_status_text(error.into());
                                    } else {
                                        app.set_status_text(
                                            format!("获取模型成功，共 {} 个模型", models.len())
                                                .into(),
                                        );
                                    }
                                }
                                Err(error) => {
                                    {
                                        let mut providers = providers_for_thread
                                            .lock()
                                            .expect("provider state poisoned");
                                        if let Some(provider) = providers.get_mut(index) {
                                            provider.models.clear();
                                        }
                                    }
                                    if app.get_current_provider_index() as usize == index {
                                        app.set_available_models(ModelRc::new(VecModel::from(
                                            Vec::<SharedString>::new(),
                                        )));
                                        app.set_current_model("".into());
                                    }
                                    let _ = persist_provider_state(
                                        &app,
                                        &providers_for_thread,
                                        &storage_for_thread,
                                    );
                                    app.set_status_text(error.into());
                                }
                            }
                        }
                    });
                });
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_select_model(move |model| {
            if let Some(app) = weak.upgrade() {
                let model_name = model.clone();
                app.set_current_model(model);
                app.set_status_text(format!("模型选择成功：{}", model_name).into());
            }
        });
    }
    {
        let providers_ref = providers.clone();
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_generate(move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            if app.get_generating() {
                return;
            }
            let model = app.get_current_model().to_string();
            if model.trim().is_empty() {
                app.set_status_text("没有选择模型，请先在 API 提供商中获取模型".into());
                return;
            }
            if let Err(error) = persist_provider_state(&app, &providers_ref, &storage_ref) {
                app.set_status_text(error.into());
                return;
            }
            let provider = Provider {
                name: app.get_current_provider_name().to_string(),
                base_url: app.get_current_base_url().to_string(),
                api_key: app.get_current_api_key().to_string(),
                model,
                models: Vec::new(),
            };
            let prompt = app.get_prompt().to_string();
            if prompt.trim().is_empty() {
                app.set_status_text("先写一句提示词吧".into());
                return;
            }
            let resolution = app.get_resolution().to_string();
            let image_format = app.get_image_format().to_string();
            let quantity = app.get_quantity().parse::<u32>().unwrap_or(1).clamp(1, 10);
            app.set_generating(true);
            app.set_status_text("正在生成图片...".into());
            let weak = app.as_weak();
            let storage_for_thread = storage_ref.clone();
            thread::spawn(move || {
                let result = generate_and_store(
                    &storage_for_thread,
                    &provider,
                    &prompt,
                    &resolution,
                    &image_format,
                    quantity,
                );
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_generating(false);
                        match result {
                            Ok(records) => {
                                let first = records.first();
                                let preview = first.and_then(|record| {
                                    storage_for_thread
                                        .image_path(&record.image_file)
                                        .ok()
                                        .and_then(|path| Image::load_from_path(&path).ok())
                                });
                                if let Err(error) = refresh_history(&app, &storage_for_thread) {
                                    app.set_status_text(error.into());
                                } else if let Some(image) = preview {
                                    app.set_preview_image(image);
                                    app.set_showing_history_preview(true);
                                    if let Some(first) = first {
                                        app.set_selected_history_id(first.id.clone().into());
                                    }
                                    app.set_status_text(
                                        format!("图片生成完成，已保存 {} 张", records.len()).into(),
                                    );
                                } else {
                                    app.set_status_text("图片已保存，但预览图片加载失败".into());
                                }
                            }
                            Err(error) => app.set_status_text(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_select_history(move |id| {
            if let Some(app) = weak.upgrade()
                && let Err(error) = select_history_record(&app, &storage_ref, &id)
            {
                app.set_status_text(error.into());
            }
        });
    }
    {
        let storage_ref = storage.clone();
        let weak = app.as_weak();
        app.on_delete_history(move |id| {
            if let Some(app) = weak.upgrade() {
                app.set_status_text("正在删除历史记录...".into());
                let weak = app.as_weak();
                let storage_for_thread = storage_ref.clone();
                thread::spawn(move || {
                    let result = storage_for_thread.delete_history(&id);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = weak.upgrade() {
                            match result {
                                Ok(Some(_)) => match refresh_history(&app, &storage_for_thread) {
                                    Ok(records) => {
                                        if app.get_selected_history_id() == id {
                                            if let Some(record) = records.first() {
                                                let _ = select_history_record(
                                                    &app,
                                                    &storage_for_thread,
                                                    &record.id,
                                                );
                                            } else {
                                                app.set_selected_history_id("".into());
                                                app.set_showing_history_preview(false);
                                                app.set_status_text("历史记录已清空".into());
                                            }
                                        }
                                        app.set_status_text("删除历史记录成功".into());
                                    }
                                    Err(error) => app.set_status_text(error.into()),
                                },
                                Ok(None) => app.set_status_text("历史记录不存在".into()),
                                Err(error) => app.set_status_text(error.into()),
                            }
                        }
                    });
                });
            }
        });
    }

    app.run()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let storage = Storage::from_executable()?;
    if std::env::args().skip(1).any(|arg| arg == "--mcp") {
        return mcp::run(storage);
    }
    run_gui(storage)
}

#[cfg(test)]
mod tests {
    use super::{endpoint, format_http_error, image_extension};

    #[test]
    fn appends_generation_path_without_duplicate_slashes() {
        assert_eq!(
            endpoint("https://api.example.com/v1", "images/generations"),
            "https://api.example.com/v1/images/generations"
        );
        assert_eq!(
            endpoint("https://api.example.com/v1/", "/images/generations"),
            "https://api.example.com/v1/images/generations"
        );
        assert_eq!(
            endpoint(
                "https://api.example.com/v1/images/generations",
                "images/generations"
            ),
            "https://api.example.com/v1/images/generations"
        );
    }

    #[test]
    fn formats_structured_http_errors() {
        let error = format_http_error(
            "图片接口请求",
            "503 Service Unavailable",
            "",
            r#"{"error":{"message":"上游服务暂时不可用"}}"#,
        );
        assert!(error.contains("HTTP 503 Service Unavailable"));
        assert!(error.contains("上游服务暂时不可用"));
    }

    #[test]
    fn formats_plain_and_empty_http_errors() {
        let plain = format_http_error("图片接口请求", "503", "", "upstream unavailable");
        assert!(plain.contains("upstream unavailable"));

        let empty = format_http_error("图片接口请求", "503", "", " ");
        assert_eq!(empty, "图片接口请求失败：HTTP 503");
    }

    #[test]
    fn detects_image_format_from_content() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nrest", "JPEG"), "png");
        assert_eq!(image_extension(&[0xff, 0xd8, 0xff, 0x00], "PNG"), "jpg");
        assert_eq!(image_extension(b"RIFFxxxxWEBPrest", "PNG"), "webp");
        assert_eq!(image_extension(b"unknown", "WEBP"), "webp");
    }
}

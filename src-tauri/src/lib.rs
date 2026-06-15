use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::path::PathBuf;
use tauri::Manager;

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("OpusMagnumRecordViewer/0.1")
        .timeout(std::time::Duration::from_secs(60))
        .pool_max_idle_per_host(1)
        .tcp_nodelay(true)
        .build()
        .expect("Failed to build HTTP client")
}

// ================= 1. ZLBB 官方生产级物理字段完美对齐 DTO =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmGroupDTO { 
    pub id: String, 
    pub display_name: String 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmPuzzleDTO { 
    pub id: String, 
    pub display_name: String, 
    pub r#type: String, 
    pub group: OmGroupDTO, 
    #[serde(default)]
    pub alt_ids: Vec<String> 
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmScoreDTO { 
    pub cost: i32, 
    pub cycles: i32, 
    pub area: i32, 
    pub instructions: i32, 
    pub overlap: bool, 
    pub trackless: bool,
    pub height: Option<i32>,
    pub width: Option<f64>,
    #[serde(rename = "boundingHex")]
    pub bounding_hex: Option<i32>,
    pub rate: Option<f64>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmRecordDTO { 
    pub id: Option<String>, 
    pub puzzle: OmPuzzleDTO, 
    pub score: Option<OmScoreDTO>, 
    pub smart_formatted_score: Option<String>, 
    pub full_formatted_score: Option<String>, 
    pub category_ids: Option<Vec<String>>, 
    pub author: Option<String>,
    pub gif: Option<String>,
    pub solution: Option<String>,
    pub last_modified: Option<String>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalSuggestion {
    pub id: String,
    pub display_name: String, 
    pub controller: String, 
}

// ================= 2. 常驻运行时状态 =================
pub struct MemoryState {
    pub record_vault: Mutex<Vec<OmRecordDTO>>,
    pub puzzle_list: Mutex<Vec<UniversalSuggestion>>,
    pub flight_lock: Mutex<Option<String>>, 
    pub boot_ready: std::sync::atomic::AtomicBool,  // 🚀 新增：启动同步完成标记
}

// ================= 3. 真实 ZLBB 数据吞吐中继引擎 =================

#[tauri::command]
async fn search_om_records(
    keyword: String,
    force: Option<bool>,
    state: tauri::State<'_, MemoryState>,
    app: tauri::AppHandle
) -> Result<Vec<OmRecordDTO>, String> {
    let input_query = keyword.trim().to_lowercase();
    if input_query.is_empty() { return Ok(vec![]); }

    // 🚀 核心优化 1：利用作用域隔离块，将 MutexGuard 的生命周期死死锁在大括号内部！
    // 这样在执行到块下方的任何 `.await` 时，list_guard 已经绝对被销毁退栈了，Future 就能安全恢复 Send 状态。
    let (controller, puzzle_id) = {
        let list_guard = state.puzzle_list.lock().unwrap();
        let matched_node = list_guard.iter()
            .find(|p| p.id.to_lowercase() == input_query || p.display_name.to_lowercase() == input_query)
            .or_else(|| {
                list_guard.iter().find(|p| p.id.to_lowercase().contains(&input_query) || p.display_name.to_lowercase().contains(&input_query))
            })
            .cloned();
            
        let ctrl = matched_node.as_ref().map(|n| n.controller.clone()).unwrap_or_else(|| "om".to_string());
        let pid = matched_node.as_ref().map(|n| n.id.clone()).unwrap_or_else(|| input_query.clone());
        (ctrl, pid)
    };

    let force_refresh = force.unwrap_or(false);

    // 🚀 核心优化 2：先查内存 Vault（非强制刷新时命中直接返回）
    if !force_refresh {
        let vault = state.record_vault.lock().unwrap();
        let total = vault.len();
        let in_memory: Vec<OmRecordDTO> = vault.iter().filter(|r| r.puzzle.id == puzzle_id).cloned().collect();
        println!("[VAULT_CHECK]: {} total records, {} for puzzle '{}'", total, in_memory.len(), puzzle_id);
        if !in_memory.is_empty() {
            println!("[VAULT_HIT]: Skipping HTTP, returning cached data.");
            return Ok(in_memory);
        }
    } else {
        println!("[FORCE_REFRESH]: Skipping memory vault, forcing API fetch.");
    }

    // 🚀 核心优化 3：并发锁占用同样执行严格大括号隔离
    {
        let mut flight = state.flight_lock.lock().unwrap();
        if let Some(current_flight) = &*flight {
            if current_flight == &puzzle_id {
                println!("[CONCURRENCY_INTERCEPT]: In-flight blocked for '{}'.", puzzle_id);
                let vault = state.record_vault.lock().unwrap();
                let cached: Vec<OmRecordDTO> = vault.iter().filter(|r| r.puzzle.id == puzzle_id).cloned().collect();
                return Ok(cached);
            }
        }
        *flight = Some(puzzle_id.clone());
    }

    let cache_dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cache_file_path = cache_dir.join(format!("{}_{}.cache.json", controller, puzzle_id));

    let app_clone = app.clone();
    let client = build_client();
    let encoded_pid = urlencoding::encode(&puzzle_id);
    let target_url = format!(
        "https://zlbb.faendir.com/{}/puzzle/{}/records?includeFrontier=true",
        controller, encoded_pid
    );

    println!("[CACHE_CHECK]: cache={} path={}", cache_file_path.exists(), cache_file_path.display());

    let mut api_error: Option<String> = None;
    let mut saved_body: Option<String> = None;

    // ── Step 1: 磁盘缓存存在且非强制刷新 → 直接用 ──
    if cache_file_path.exists() && !force_refresh {
        match std::fs::read_to_string(&cache_file_path) {
            Ok(json) => {
                println!("[CACHE_HIT]: Loaded {} bytes from disk.", json.len());
                saved_body = Some(json);
            }
            Err(e) => eprintln!("[CACHE_ERROR]: {}", e),
        }
    }

    // ── Step 2: 无缓存或缓存过期 → 联网下载 ──
    if saved_body.is_none() {
        println!("[API_FETCH]: {}", target_url);
        for attempt in 1..=2 {
            match client.get(&target_url).send().await {
                Ok(response) if response.status().is_success() => {
                    let ce = response.headers().get("content-encoding").map(|v| v.to_str().unwrap_or("?")).unwrap_or("none");
                    println!("[API_DOWNLOAD]: Content-Encoding={}, attempt={}", ce, attempt);
                    match response.bytes().await {
                        Ok(bytes) => {
                            let len = bytes.len();
                            println!("[API_DOWNLOAD]: Received {} bytes", len);
                            saved_body = Some(String::from_utf8_lossy(&bytes).to_string());
                            // 写入磁盘缓存
                            if let Err(e) = std::fs::write(&cache_file_path, saved_body.as_ref().unwrap()) {
                                eprintln!("[CACHE_WRITE_ERROR]: {}", e);
                            } else {
                                println!("[CACHE_WRITE]: Saved {} bytes to disk.", len);
                                save_cache_meta(&app_clone);
                            }
                            break;
                        }
                        Err(e) => {
                            api_error = Some(format!("Failed to read body: {}", e));
                            eprintln!("[API_ERROR]: Attempt {} failed: {}", attempt, e);
                        }
                    }
                }
                Ok(response) => {
                    let status = response.status();
                    println!("[API_ERROR]: HTTP {}", status);
                    api_error = Some(format!("Server returned HTTP {}", status));
                    break;
                }
                Err(e) => {
                    println!("[API_ERROR]: Connection failed: {}", e);
                    api_error = Some(format!("Network request failed: {}", e));
                    if attempt < 2 { println!("[API_RETRY]: Retrying..."); }
                }
            }
        }
    }

    if let Some(body) = saved_body {
        match serde_json::from_str::<Vec<OmRecordDTO>>(&body) {
            Ok(remote_records) => {
                println!("[ZLBB_PARSER]: {} records from API.", remote_records.len());
                {
                    let memory_state = app_clone.state::<MemoryState>();
                    let mut vault = memory_state.record_vault.lock().unwrap();
                    for remote in &remote_records {
                        let exists = vault.iter().any(|local|
                            local.puzzle.id == remote.puzzle.id && local.full_formatted_score == remote.full_formatted_score
                        );
                        if !exists {
                            vault.push(remote.clone());
                        }
                    }
                    // 缓存已在下载时写入原始字节，这里只更新内存 vault
                }
                {
                    let mut flight = state.flight_lock.lock().unwrap();
                    *flight = None;
                }
                let vault = state.record_vault.lock().unwrap();
                let results: Vec<OmRecordDTO> = vault.iter()
                    .filter(|r| r.puzzle.id == puzzle_id)
                    .cloned()
                    .collect();
                println!("[RESULT]: API returned {} records.", results.len());
                return Ok(results);
            }
            Err(e) => {
                let preview = &body[..body.len().min(300)];
                api_error = Some(format!("JSON error at line {} col {}: {}.  Body: {}...", e.line(), e.column(), e, preview));
                eprintln!("[ZLBB_ERROR]: {}", api_error.as_ref().unwrap());
            }
        }
    }

    {
        let mut flight = state.flight_lock.lock().unwrap();
        *flight = None;
    }

    let final_vault = state.record_vault.lock().unwrap();
    let final_results: Vec<OmRecordDTO> = final_vault
        .iter()
        .filter(|r| r.puzzle.id == puzzle_id)
        .cloned()
        .collect();

    if final_results.is_empty() {
        if let Some(err) = api_error {
            return Err(err);
        }
    }

    println!("[RESULT]: Returning {} records.", final_results.len());
    Ok(final_results)
}

// ================= 4. 缓存信息查询 =================

#[tauri::command]
fn get_cache_path(app: tauri::AppHandle) -> String {
    app.path().app_cache_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| "unknown".to_string())
}

#[tauri::command]
fn get_cache_info(app: tauri::AppHandle) -> String {
    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
    let meta_path = dir.join("cache_meta.json");
    let local = std::fs::read_to_string(&meta_path).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("updated").cloned())
        .and_then(|v| v.as_str().map(|s| format!("Local: {}", s)))
        .unwrap_or_else(|| {
            let now = utc_now();
            let _ = std::fs::write(&meta_path, format!("{{\"updated\":\"{}\"}}", now));
            format!("Local: {} (new)", now)
        });
    local
}

fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn save_cache_meta(app: &tauri::AppHandle) {
    if let Ok(dir) = app.path().app_cache_dir() {
        let now = utc_now();
        let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\"updated\":\"{}\"}}", now));
    }
}


// ================= 5. ZLBB 跨游戏模糊检索提示命令 =================

#[tauri::command]
async fn get_live_puzzle_suggestions(
    keyword: String,
    state: tauri::State<'_, MemoryState>
) -> Result<Vec<UniversalSuggestion>, String> {
    let fragment = keyword.trim().to_lowercase();
    if fragment.is_empty() { return Ok(vec![]); }

    let list = state.puzzle_list.lock().unwrap();
    let matches: Vec<UniversalSuggestion> = list
        .iter()
        .filter(|p| p.id.to_lowercase().contains(&fragment) || p.display_name.to_lowercase().contains(&fragment))
        .cloned()
        .collect();

    Ok(matches)
}

// ================= 5. 启动同步状态查询 =================

#[tauri::command]
fn check_boot_ready(state: tauri::State<'_, MemoryState>) -> bool {
    state.boot_ready.load(std::sync::atomic::Ordering::Acquire)
}

// ================= 6. App 启动阶段 =================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(MemoryState {
                record_vault: Mutex::new(Vec::new()),
                puzzle_list: Mutex::new(Vec::new()),
                flight_lock: Mutex::new(None),
                boot_ready: std::sync::atomic::AtomicBool::new(false),
            });

            let handle = app.handle().clone();
            let cache_dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
            println!("[BOOT]: Cache directory: {}", cache_dir.display());
            let puzzle_cache_path = cache_dir.join("puzzles.cache.json");

            tauri::async_runtime::spawn(async move {
                let client = build_client();
                let base_api = "https://zlbb.faendir.com";
                let mut aggregated = Vec::new();
                let mut api_ok = false;
                
                // ── 优先加载磁盘缓存（毫秒级）──
                if puzzle_cache_path.exists() {
                    if let Ok(json) = std::fs::read_to_string(&puzzle_cache_path) {
                        if let Ok(cached) = serde_json::from_str::<Vec<UniversalSuggestion>>(&json) {
                            println!("[BOOT_CACHE]: Loaded {} puzzles from disk.", cached.len());
                            let ms = handle.state::<MemoryState>();
                            let mut list = ms.puzzle_list.lock().unwrap();
                            *list = cached;
                            ms.boot_ready.store(true, std::sync::atomic::Ordering::Release);
                            println!("[BOOT_READY]: App ready (cached).");
                        }
                    }
                }
                // ── 后台更新 ──
                match client.get(format!("{}/om/puzzles", base_api)).send().await {
                    Ok(res) => {
                        match res.json::<Vec<OmPuzzleDTO>>().await {
                            Ok(puzzles) => {
                                let count = puzzles.len();
                                for p in puzzles {
                                    aggregated.push(UniversalSuggestion { id: p.id, display_name: p.display_name, controller: "om".to_string() });
                                }
                                println!("[BOOT_SYNC]: OM puzzles loaded ({} entries).", count);
                                api_ok = true;
                            }
                            Err(e) => eprintln!("[BOOT_ERROR]: Failed to parse /om/puzzles JSON: {}", e),
                        }
                    }
                    Err(e) => eprintln!("[BOOT_ERROR]: Failed to fetch /om/puzzles: {}", e),
                }
                match client.get(format!("{}/exa/puzzles", base_api)).send().await {
                    Ok(res) => {
                        match res.json::<Vec<OmPuzzleDTO>>().await {
                            Ok(puzzles) => {
                                let count = puzzles.len();
                                for p in puzzles {
                                    aggregated.push(UniversalSuggestion { id: p.id, display_name: p.display_name, controller: "exa".to_string() });
                                }
                                println!("[BOOT_SYNC]: EXA puzzles loaded ({} entries).", count);
                                api_ok = true;
                            }
                            Err(e) => eprintln!("[BOOT_ERROR]: Failed to parse /exa/puzzles JSON: {}", e),
                        }
                    }
                    Err(e) => eprintln!("[BOOT_ERROR]: Failed to fetch /exa/puzzles: {}", e),
                }

                // API 成功 → 写入磁盘缓存
                if api_ok && !aggregated.is_empty() {
                    if let Ok(json) = serde_json::to_string(&aggregated) {
                        if let Err(e) = std::fs::write(&puzzle_cache_path, &json) {
                            eprintln!("[CACHE_ERROR]: Failed to write puzzle cache: {}", e);
                        } else {
                            println!("[CACHE_SAVED]: Puzzle list cached ({} entries).", aggregated.len());
                            if let Ok(dir) = handle.path().app_cache_dir() {
                                let now = utc_now();
                                let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\"updated\":\"{}\"}}", now));
                            }
                        }
                    }
                }

                // API 全部失败 → 回退磁盘缓存
                if aggregated.is_empty() && puzzle_cache_path.exists() {
                    if let Ok(file_content) = std::fs::read_to_string(&puzzle_cache_path) {
                        if let Ok(cached) = serde_json::from_str::<Vec<UniversalSuggestion>>(&file_content) {
                            aggregated = cached;
                            println!("[CACHE_HIT]: Puzzle list loaded from disk ({} entries).", aggregated.len());
                        }
                    }
                }

                let len = aggregated.len();
                let memory_state = handle.state::<MemoryState>();
                let mut list = memory_state.puzzle_list.lock().unwrap();
                *list = aggregated;
                memory_state.boot_ready.store(true, std::sync::atomic::Ordering::Release);
                println!("[BOOT_SYNC]: Global maps initialized ({} puzzles).", len);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![search_om_records, get_live_puzzle_suggestions, check_boot_ready, get_cache_path, get_cache_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
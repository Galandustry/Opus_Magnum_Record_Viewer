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
    #[serde(default)]
    pub cost: i32, 
    #[serde(default)]
    pub cycles: i32, 
    #[serde(default)]
    pub area: i32, 
    #[serde(default)]
    pub instructions: i32, 
    #[serde(default)]
    pub overlap: bool, 
    #[serde(default)]
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
    #[serde(default)]
    pub puzzle: Option<OmPuzzleDTO>, 
    pub score: Option<OmScoreDTO>, 
    pub smart_formatted_score: Option<String>, 
    pub full_formatted_score: Option<String>, 
    pub category_ids: Option<Vec<String>>, 
    pub smart_formatted_categories: Option<String>, 
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
        let in_memory: Vec<OmRecordDTO> = vault.iter().filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)).cloned().collect();
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
                let cached: Vec<OmRecordDTO> = vault.iter().filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)).cloned().collect();
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
        let mut remote_records: Vec<OmRecordDTO> = Vec::new();
        let mut raw_count = 0usize;
        let mut skipped = 0usize;
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
            raw_count = values.len();
            if let Some(first) = values.first() {
                let preview = serde_json::to_string(first).unwrap_or_default();
                println!("[ZLBB_DEBUG]: first record preview: {}...", &preview[..preview.len().min(200)]);
            }
            for (i, v) in values.into_iter().enumerate() {
                match serde_json::from_value::<OmRecordDTO>(v) {
                    Ok(record) => remote_records.push(record),
                    Err(e) => { skipped += 1; if skipped <= 3 { eprintln!("[ZLBB_SKIP]: record[{}] parse failed: {}", i, e); } }
                }
            }
        } else {
            match serde_json::from_str::<Vec<OmRecordDTO>>(&body) {
                Ok(records) => { raw_count = records.len(); remote_records = records; }
                Err(e) => {
                    let preview = &body[..body.len().min(300)];
                    api_error = Some(format!("JSON error: {}. Body: {}...", e, preview));
                    eprintln!("[ZLBB_ERROR]: {}", api_error.as_ref().unwrap());
                }
            }
        }
        println!("[ZLBB_PARSER]: {} raw, {} parsed, {} skipped", raw_count, remote_records.len(), skipped);
        if !remote_records.is_empty() {
            {
                let memory_state = app_clone.state::<MemoryState>();
                let puzzle_dto: Option<OmPuzzleDTO> = {
                    let list = memory_state.puzzle_list.lock().unwrap();
                    list.iter().find(|s| s.id == puzzle_id).map(|s| OmPuzzleDTO {
                        id: s.id.clone(), display_name: s.display_name.clone(),
                        r#type: String::new(), group: OmGroupDTO { id: String::new(), display_name: String::new() },
                        alt_ids: Vec::new(),
                    })
                };
                let mut vault = memory_state.record_vault.lock().unwrap();
                for mut remote in remote_records {
                    if remote.puzzle.is_none() { remote.puzzle = puzzle_dto.clone(); }
                    let exists = vault.iter().any(|local|
                        local.puzzle.as_ref().zip(remote.puzzle.as_ref()).map_or(false, |(lp, rp)| lp.id == rp.id)
                            && local.full_formatted_score == remote.full_formatted_score
                    );
                    if !exists { vault.push(remote); }
                }
            }
            { let mut flight = state.flight_lock.lock().unwrap(); *flight = None; }
            let vault = state.record_vault.lock().unwrap();
            let results: Vec<OmRecordDTO> = vault.iter()
                .filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id))
                .cloned().collect();
            println!("[RESULT]: returning {} records for puzzle '{}'.", results.len(), puzzle_id);
            return Ok(results);
        } else if let Some(err) = api_error { return Err(err); }
    }

    let final_vault = state.record_vault.lock().unwrap();
    let final_results: Vec<OmRecordDTO> = final_vault
        .iter()
        .filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id))
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

// ================= Pareto 判定面板类型 =================

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum InputMode { GCA, GCAI }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ParetoJudgeStatus { Ok, Unknown, UnknownBreaking, AlreadyPresented, NothingBeaten }

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmDraftInput {
    pub cost: Option<i32>, pub cycles: Option<i32>,
    pub area: Option<i32>, pub instructions: Option<i32>,
    pub height: Option<i32>, pub width: Option<f64>,
    pub bounding_hex: Option<i32>, pub rate: Option<f64>,
    pub overlap: bool, pub trackless: bool,
    #[serde(rename = "active_metrics")]
    pub active_metrics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatenMetricDiff {
    pub actual_value: i32, pub absolute_diff: i32,
    pub percentage_diff: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParetoBeatenReport {
    pub better_record: OmRecordDTO,
    pub cost_diff: Option<BeatenMetricDiff>, pub cycles_diff: Option<BeatenMetricDiff>,
    pub area_diff: Option<BeatenMetricDiff>, pub instructions_diff: Option<BeatenMetricDiff>,
    pub height_diff: Option<BeatenMetricDiff>, pub width_diff: Option<BeatenMetricDiff>,
    pub bhex_diff: Option<BeatenMetricDiff>, pub rate_diff: Option<BeatenMetricDiff>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeResult {
    pub status: ParetoJudgeStatus, pub total_compared: usize,
    pub reports: Vec<ParetoBeatenReport>,
}

// ================= BEST 标杆持久化 =================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PuzzleBestMetrics {
    pub puzzle_id: String,
    pub cost: i32, pub cycles: i32, pub area: i32, pub instructions: i32,
    pub height: i32, pub width: f64, pub bounding_hex: i32, pub rate: f64,
}

#[tauri::command]
fn save_puzzle_best(metrics: PuzzleBestMetrics, app: tauri::AppHandle) {
    if let Ok(dir) = app.path().app_cache_dir() {
        let path = dir.join(format!("best_{}.cache.json", metrics.puzzle_id));
        if let Ok(json) = serde_json::to_string(&metrics) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[tauri::command]
fn load_puzzle_best(puzzle_id: String, app: tauri::AppHandle) -> Option<PuzzleBestMetrics> {
    let dir = app.path().app_cache_dir().ok()?;
    let path = dir.join(format!("best_{}.cache.json", puzzle_id));
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

// ================= 增量同步引擎 =================

#[tauri::command]
async fn sync_incremental(
    since: Option<String>, controller: Option<String>,
    state: tauri::State<'_, MemoryState>, app: tauri::AppHandle
) -> Result<SyncResult, String> {
    let ctrl = controller.unwrap_or_else(|| "om".to_string());
    let mut errors: Vec<String> = Vec::new();
    let mut added = 0usize; let mut removed = 0usize;
    // 清洗时间戳：兼容历史脏数据，统一转为秒级 ISO 8601 (Z结尾)
    let raw_since = since.or_else(|| read_last_sync_time(&app))
        .unwrap_or_else(|| utc_now_iso());
    let clean_since = chrono::DateTime::parse_from_rfc3339(&raw_since)
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|_| utc_now_iso());
    println!("[SYNC]: ctrl={} since={}", ctrl, clean_since);
    let client = build_client(); let base = "https://zlbb.faendir.com";
    let enc = urlencoding::encode(&clean_since);
    {   let url = format!("{}/{}/records/new/{}", base, ctrl, enc);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw=0usize; let mut parsed: Vec<OmRecordDTO>=Vec::new();
                if let Ok(body)=res.text().await { if let Ok(vals)=serde_json::from_str::<Vec<serde_json::Value>>(&body) { raw=vals.len(); for v in vals { if let Ok(r)=serde_json::from_value::<OmRecordDTO>(v) { parsed.push(r); } } } }
                if !parsed.is_empty() { let mut vault=state.record_vault.lock().unwrap(); for r in &parsed { if !vault.iter().any(|v| v.id.as_deref()==r.id.as_deref()&&v.full_formatted_score.as_deref()==r.full_formatted_score.as_deref()) { vault.push(r.clone()); added+=1; } } }
                println!("[SYNC_NEW]: {} raw {} added", raw, added); }
            Ok(res) => errors.push(format!("new HTTP {}", res.status())),
            Err(e) => errors.push(format!("new fetch: {}", e)),
        }
    }
    {   let url = format!("{}/{}/records/changes/{}", base, ctrl, enc);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw=0usize; let mut parsed: Vec<OmRecordChange>=Vec::new();
                if let Ok(body)=res.text().await { if let Ok(vals)=serde_json::from_str::<Vec<serde_json::Value>>(&body) { raw=vals.len(); for v in vals { if let Ok(ch)=serde_json::from_value::<OmRecordChange>(v) { parsed.push(ch); } } } }
                let mut vault=state.record_vault.lock().unwrap();
                for ch in &parsed { match ch.r#type.as_str() { "ADD"=>{ if !vault.iter().any(|v| v.id.as_deref()==ch.record.id.as_deref()&&v.full_formatted_score.as_deref()==ch.record.full_formatted_score.as_deref()) { vault.push(ch.record.clone()); added+=1; } } "REMOVE"=>{ if let Some(ref rid)=ch.record.id { let b=vault.len(); vault.retain(|v| v.id.as_deref()!=Some(rid.as_str())); removed+=b-vault.len(); } } _=>{} } }
                println!("[SYNC_CHANGES]: {} raw", raw); }
            Ok(res) => errors.push(format!("changes HTTP {}", res.status())),
            Err(e) => errors.push(format!("changes fetch: {}", e)),
        }
    }
    let synced_until=utc_now_iso(); save_cache_meta(&app);
    println!("[SYNC_DONE]: +{} -{} until={}", added, removed, synced_until);
    Ok(SyncResult { new_count: added, removed_count: removed, synced_until, errors })
}

// ================= Pareto 实时判定引擎（重构版） =================

/// 提取 Draft 与 Record 在某个指定维度上的数值对
fn extract_metric_pair(dim: &str, draft: &OmDraftInput, score: &OmScoreDTO) -> (Option<f64>, Option<f64>) {
    match dim {
        "cost" => (draft.cost.map(|v| v as f64), Some(score.cost as f64)),
        "cycles" => (draft.cycles.map(|v| v as f64), Some(score.cycles as f64)),
        "area" => (draft.area.map(|v| v as f64), Some(score.area as f64)),
        "instructions" => (draft.instructions.map(|v| v as f64), Some(score.instructions as f64)),
        "height" => (draft.height.map(|v| v as f64), score.height.map(|v| v as f64)),
        "width" => (draft.width, score.width),
        "boundingHex" => (draft.bounding_hex.map(|v| v as f64), score.bounding_hex.map(|v| v as f64)),
        "rate" => (draft.rate, score.rate),
        _ => (None, None),
    }
}

/// 核心算法 1：历史记录是否【绝对支配】Draft
/// 在 Draft 已填维度上，Record 全方位不亚于 Draft 且至少一项严格更优。
/// 只要 Draft 在任一维度更好 → 立即返回 false（Record 无法支配）。
fn is_record_dominating_draft(draft: &OmDraftInput, score: &OmScoreDTO, dims: &[&str]) -> bool {
    let mut record_has_better = false;
    let mut any_compared = false;
    for &dim in dims {
        match extract_metric_pair(dim, draft, score) {
            (Some(dv), Some(rv)) => {
                any_compared = true;
                // Draft 在此维度更好 → Record 瞬间丧失支配资格
                if dv < rv - 0.00001 { return false; }
                if rv < dv - 0.00001 { record_has_better = true; }
            }
            // Draft 填了但记录缺项 → Draft 占优 → Record 无法支配
            (Some(_), None) => return false,
            // Draft 没填的维度 → 不参与支配评估
            _ => {}
        }
    }
    any_compared && record_has_better
}

/// 核心算法 2：Draft 是否在任一维度打破了该维度的全局历史极限
fn does_draft_break_global_best(draft: &OmDraftInput, dims: &[&str], candidates: &[OmRecordDTO]) -> bool {
    for &dim in dims {
        let d_val = match dim {
            "cost" => draft.cost.map(|v| v as f64),
            "cycles" => draft.cycles.map(|v| v as f64),
            "area" => draft.area.map(|v| v as f64),
            "instructions" => draft.instructions.map(|v| v as f64),
            "height" => draft.height.map(|v| v as f64),
            "width" => draft.width,
            "boundingHex" => draft.bounding_hex.map(|v| v as f64),
            "rate" => draft.rate,
            _ => None,
        };
        if let Some(dv) = d_val {
            let mut global_best = f64::MAX;
            let mut has_valid = false;
            for r in candidates {
                if let Some(s) = &r.score {
                    if let (_, Some(rv)) = extract_metric_pair(dim, draft, s) {
                        has_valid = true;
                        if rv < global_best { global_best = rv; }
                    }
                }
            }
            if has_valid && dv < global_best - 0.00001 { return true; }
        }
    }
    false
}

/// 核心算法 3：Draft 是否与某条历史记录在已填维度上完全相同
fn is_exact_match(draft: &OmDraftInput, score: &OmScoreDTO, dims: &[&str]) -> bool {
    for &dim in dims {
        match extract_metric_pair(dim, draft, score) {
            (Some(dv), Some(rv)) => {
                if (dv - rv).abs() > 0.00001 { return false; }
            }
            (None, None) => {}
            _ => return false,
        }
    }
    true
}

fn build_beaten_report(draft: &OmDraftInput, better: &OmRecordDTO) -> ParetoBeatenReport {
    let s = better.score.as_ref().unwrap();
    let ci32 = |dv: Option<i32>, rv: i32| -> Option<BeatenMetricDiff> {
        let dv = dv?;
        Some(BeatenMetricDiff { actual_value: rv, absolute_diff: dv - rv, percentage_diff: (dv as f64)/(rv as f64)*100.0 })
    };
    let cf64 = |dv: Option<f64>, rv: f64| -> Option<BeatenMetricDiff> {
        let dv = dv?;
        Some(BeatenMetricDiff { actual_value: rv as i32, absolute_diff: (dv - rv) as i32, percentage_diff: (dv/rv)*100.0 })
    };
    ParetoBeatenReport {
        better_record: better.clone(),
        cost_diff: ci32(draft.cost, s.cost), cycles_diff: ci32(draft.cycles, s.cycles),
        area_diff: ci32(draft.area, s.area), instructions_diff: ci32(draft.instructions, s.instructions),
        height_diff: s.height.and_then(|rv| ci32(draft.height, rv)),
        width_diff: s.width.and_then(|rv| cf64(draft.width, rv)),
        bhex_diff: s.bounding_hex.and_then(|rv| ci32(draft.bounding_hex, rv)),
        rate_diff: s.rate.and_then(|rv| cf64(draft.rate, rv)),
    }
}

// ================= 主入口：Pareto 判定漏斗 =================

#[tauri::command]
async fn judge_draft(
    draft: OmDraftInput,
    puzzle_id: String,
    state: tauri::State<'_, MemoryState>,
) -> Result<JudgeResult, String> {
    let dims: Vec<&str> = draft.active_metrics.iter().map(|s| s.as_str()).collect();
    if dims.is_empty() {
        return Ok(JudgeResult { status: ParetoJudgeStatus::Unknown, total_compared: 0, reports: vec![] });
    }

    // 1. 统计已填维度
    let expected = dims.len();
    let filled = dims.iter().filter(|&&dim| match dim {
        "cost" => draft.cost.is_some(),
        "cycles" => draft.cycles.is_some(),
        "area" => draft.area.is_some(),
        "instructions" => draft.instructions.is_some(),
        "height" => draft.height.is_some(),
        "width" => draft.width.is_some(),
        "boundingHex" => draft.bounding_hex.is_some(),
        "rate" => draft.rate.is_some(),
        _ => false,
    }).count();
    let is_all_filled = expected == filled;
    if filled == 0 {
        return Ok(JudgeResult { status: ParetoJudgeStatus::Unknown, total_compared: 0, reports: vec![] });
    }

    // 2. 捞取当前关卡的有效对手 → clone 后立即释放锁，防止阻塞
    let candidates: Vec<OmRecordDTO> = {
        let vault = state.record_vault.lock().unwrap();
        vault.iter().filter(|r| {
            r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)
                && r.score.is_some()
                && {
                    let s = r.score.as_ref().unwrap();
                    let overlap_match = s.overlap == draft.overlap;
                    let trackless_match = if draft.trackless { s.trackless } else { true };
                    overlap_match && trackless_match
                }
        }).cloned().collect()
    }; // ← 锁在此处释放
    let total = candidates.len();

    if total == 0 {
        return Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: 0, reports: vec![] });
    }

    // 3. 【第一关】支配力筛查 — 只要有一条记录全面压制 Draft，就是 NothingBeaten
    let mut dominators: Vec<&OmRecordDTO> = Vec::new();
    for r in &candidates {
        let s = r.score.as_ref().unwrap();
        if is_record_dominating_draft(&draft, s, &dims) {
            dominators.push(r);
        }
    }
    if !dominators.is_empty() {
        let reports: Vec<ParetoBeatenReport> = dominators.iter()
            .map(|d| build_beaten_report(&draft, d)).collect();
        return Ok(JudgeResult { status: ParetoJudgeStatus::NothingBeaten, total_compared: total, reports });
    }

    // 4. 【第二关】未填满 → 检查是否打破全局极限
    if !is_all_filled {
        let breaking = does_draft_break_global_best(&draft, &dims, &candidates);
        if breaking {
            return Ok(JudgeResult { status: ParetoJudgeStatus::UnknownBreaking, total_compared: total, reports: vec![] });
        }
        return Ok(JudgeResult { status: ParetoJudgeStatus::Unknown, total_compared: total, reports: vec![] });
    }

    // 5. 【第三关】已填满、未被支配 → 检查是否撞车
    let exact_match = candidates.iter().any(|r| {
        is_exact_match(&draft, r.score.as_ref().unwrap(), &dims)
    });
    if exact_match {
        return Ok(JudgeResult { status: ParetoJudgeStatus::AlreadyPresented, total_compared: total, reports: vec![] });
    }

    // 6. 填满 + 未被支配 + 未撞车 → 帕累托前沿突破！
    Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: total, reports: vec![] })
}

// ================= 5. ZLBB 跨游戏模糊检索提示命令 =================

#[tauri::command]
fn get_cache_info(app: tauri::AppHandle) -> String {
    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
    let meta_path = dir.join("cache_meta.json");
    let local = std::fs::read_to_string(&meta_path).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("updated").cloned())
        .and_then(|v| v.as_str().map(|s| format!("Local: {}", s)))
        .unwrap_or_else(|| {
            let now = utc_now_iso();
            let _ = std::fs::write(&meta_path, format!("{{\"updated\":\"{}\"}}", now));
            format!("Local: {} (new)", utc_now())
        });
    local
}

fn utc_now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn utc_now() -> String {
    chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn save_cache_meta(app: &tauri::AppHandle) {
    if let Ok(dir) = app.path().app_cache_dir() {
        let now = utc_now_iso();
        let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\"updated\":\"{}\"}}", now));
    }
}

fn read_last_sync_time(app: &tauri::AppHandle) -> Option<String> {
    let dir = app.path().app_cache_dir().ok()?;
    let meta_path = dir.join("cache_meta.json");
    let content = std::fs::read_to_string(&meta_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("updated")?.as_str().map(|s| s.to_string())
}

// ================= 增量同步结构 =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmRecordChange {
    pub r#type: String,
    pub record: OmRecordDTO,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub new_count: usize, pub removed_count: usize,
    pub synced_until: String, pub errors: Vec<String>,
}


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
        .invoke_handler(tauri::generate_handler![search_om_records, sync_incremental, judge_draft, save_puzzle_best, load_puzzle_best, get_live_puzzle_suggestions, check_boot_ready, get_cache_path, get_cache_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
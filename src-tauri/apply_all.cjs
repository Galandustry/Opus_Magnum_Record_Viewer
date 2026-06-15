// apply_all.js — 一次性应用所有重构变更到 lib.rs
const fs = require('fs');
let c = fs.readFileSync('src/lib.rs', 'utf8');

// ═══ 1. OmScoreDTO: 必填字段加 #[serde(default)] ═══
c = c.replace('    pub cost: i32,', '    #[serde(default)]\n    pub cost: i32,');
c = c.replace('    pub cycles: i32,', '    #[serde(default)]\n    pub cycles: i32,');
c = c.replace('    pub area: i32,', '    #[serde(default)]\n    pub area: i32,');
c = c.replace('    pub instructions: i32,', '    #[serde(default)]\n    pub instructions: i32,');
c = c.replace('    pub overlap: bool,', '    #[serde(default)]\n    pub overlap: bool,');
c = c.replace('    pub trackless: bool,', '    #[serde(default)]\n    pub trackless: bool,');

// ═══ 2. OmScoreDTO: 添加 INF 字段 ═══
c = c.replace(
    '    pub rate: Option<f64>\n}',
    '    pub rate: Option<f64>,\n    // 无限关卡极限指标\n    pub area_inf_level: Option<i32>,\n    pub area_inf_value: Option<f64>,\n    pub height_inf: Option<f64>,\n    pub width_inf: Option<f64>,\n    #[serde(rename = "boundingHexINF")]\n    pub bounding_hex_inf: Option<f64>,\n}'
);

// ═══ 3. OmRecordDTO: puzzle 改 Optional, 加 smart_formatted_categories ═══
c = c.replace(
    '    pub puzzle: OmPuzzleDTO,',
    '    #[serde(default)]\n    pub puzzle: Option<OmPuzzleDTO>,'
);
c = c.replace(
    '    pub category_ids: Option<Vec<String>>,',
    '    pub category_ids: Option<Vec<String>>, \n    pub smart_formatted_categories: Option<String>,'
);

// ═══ 4. 插入新类型: OmRecordChange, SyncResult, Pareto 系列 ═══
const marker4 = '    pub controller: String, \n}\n\n// ================= 2. 常驻运行时状态 =================';
const extraTypes = `
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
    pub new_count: usize,
    pub removed_count: usize,
    pub synced_until: String,
    pub errors: Vec<String>,
}

// ================= Pareto 判定面板类型 =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputMode { GCA, GCAI }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParetoJudgeStatus { Ok, Unknown, AlreadyPresented, NothingBeaten }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmDraftInput {
    pub cost: Option<i32>, pub cycles: Option<i32>,
    pub area: Option<i32>, pub instructions: Option<i32>,
    pub overlap: bool, pub trackless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatenMetricDiff {
    pub actual_value: i32, pub absolute_diff: i32,
    pub percentage_diff: f64, pub formatted_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParetoBeatenReport {
    pub better_record: OmRecordDTO,
    pub cost_diff: Option<BeatenMetricDiff>, pub cycles_diff: Option<BeatenMetricDiff>,
    pub area_diff: Option<BeatenMetricDiff>, pub instructions_diff: Option<BeatenMetricDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeResult {
    pub status: ParetoJudgeStatus, pub total_compared: usize,
    pub report: Option<ParetoBeatenReport>,
}
`;
if (!c.includes('pub struct OmRecordChange')) {
    c = c.replace(marker4, '    pub controller: String, \n}' + extraTypes + '\n// ================= 2. 常驻运行时状态 =================');
}

// ═══ 5. 全局替换 .puzzle.id → .puzzle.as_ref().map_or(false, |p| p.id == puzzle_id) ═══
// 用正则替换所有形如 r.puzzle.id == puzzle_id 的模式
c = c.replace(/\.puzzle\.id\s*==\s*puzzle_id/g, '.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)');

// 特殊处理双 puzzle 比较
c = c.replace(
    'local.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id) == remote.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)',
    'local.puzzle.as_ref().zip(remote.puzzle.as_ref()).map_or(false, |(lp, rp)| lp.id == rp.id)'
);

// ═══ 6. 替换 search_om_records 中的解析块 → 弹性解析 + puzzle 注入 ═══
const oldBlock = `    if let Some(body) = saved_body {
        match serde_json::from_str::<Vec<OmRecordDTO>>(&body) {
            Ok(remote_records) => {
                println!("[ZLBB_PARSER]: {} records from API.", remote_records.len());
                {
                    let memory_state = app_clone.state::<MemoryState>();
                    let mut vault = memory_state.record_vault.lock().unwrap();
                    for remote in &remote_records {
                        let exists = vault.iter().any(|local|
                            local.puzzle.as_ref().zip(remote.puzzle.as_ref()).map_or(false, |(lp, rp)| lp.id == rp.id)
                                && local.full_formatted_score == remote.full_formatted_score
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
                    .filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id))
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
    }`;

const newBlock = `    if let Some(body) = saved_body {
        // ── 弹性解析：逐条反序列化，单条损坏不影响整批 ──
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
    }`;

if (c.includes(oldBlock)) {
    c = c.replace(oldBlock, newBlock);
    console.log('✓ Block 6 replaced');
} else {
    console.log('✗ Block 6 NOT FOUND — using fallback anchor');
    // 用关键行定位
    const anchor = 'if let Some(body) = saved_body {';
    const tailMarker = '    let final_vault = state.record_vault.lock().unwrap();';
    const ai = c.indexOf(anchor);
    const ti = c.indexOf(tailMarker, ai);
    if (ai >= 0 && ti >= 0) {
        c = c.substring(0, ai) + newBlock + '\n\n' + c.substring(ti);
        console.log('✓ Block 6 replaced via anchors');
    } else {
        console.log('✗ anchors not found: ai=' + ai + ' ti=' + ti);
    }
}

// ═══ 7. 替换 get_cache_info 使用 ISO 8601 ═══
c = c.replace(
    'fn utc_now() -> String {',
    'fn utc_now_iso() -> String {\n    chrono::Utc::now().to_rfc3339()\n}\n\nfn utc_now() -> String {'
);
c = c.replace(
    'let now = utc_now();\n                let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));',
    'let now = utc_now_iso();\n                let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));'
);
// Update get_cache_info to parse ISO 8601
const oldGetCacheInfo = `fn get_cache_info(app: tauri::AppHandle) -> String {
    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
    let meta_path = dir.join("cache_meta.json");
    let local = std::fs::read_to_string(&meta_path).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("updated").cloned())
        .and_then(|v| v.as_str().map(|s| format!("Local: {}", s)))
        .unwrap_or_else(|| {
            let now = utc_now();
            let _ = std::fs::write(&meta_path, format!("{{\\"updated\\":\\"{}\\"}}", now));
            format!("Local: {} (new)", now)
        });
    local
}`;

const newGetCacheInfo = `fn get_cache_info(app: tauri::AppHandle) -> String {
    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));
    let meta_path = dir.join("cache_meta.json");
    let local = std::fs::read_to_string(&meta_path).ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("updated").cloned())
        .and_then(|v| v.as_str().map(|raw| {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
                format!("Local: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"))
            } else {
                format!("Local: {}", raw)
            }
        }))
        .unwrap_or_else(|| {
            let now = utc_now_iso();
            let _ = std::fs::write(&meta_path, format!("{{\\"updated\\":\\"{}\\"}}", now));
            format!("Local: {} (new)", utc_now())
        });
    local
}`;

if (c.includes('fn get_cache_info')) {
    c = c.replace(oldGetCacheInfo, newGetCacheInfo);
    console.log('✓ get_cache_info updated');
}

// ═══ 8. 在其他 save_cache_meta 调用点更新为 ISO ═══
// 启动区的 cache_meta 写入
c = c.replace(
    'if let Ok(dir) = handle.path().app_cache_dir() {\n                                let now = utc_now();\n                                let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));',
    'if let Ok(dir) = handle.path().app_cache_dir() {\n                                let now = utc_now_iso();\n                                let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));'
);

// ═══ 9. 添加 sync_incremental 和 judge_draft 命令, 以及 read_last_sync_time ═══
// 在 save_cache_meta 之后插入新函数和命令
const syncAndJudge = `
fn read_last_sync_time(app: &tauri::AppHandle) -> Option<String> {
    let dir = app.path().app_cache_dir().ok()?;
    let meta_path = dir.join("cache_meta.json");
    let content = std::fs::read_to_string(&meta_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("updated")?.as_str().map(|s| s.to_string())
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
    let since = since.or_else(|| read_last_sync_time(&app))
        .unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339());
    println!("[SYNC]: controller={}, since={}", ctrl, since);
    let client = build_client();
    let base = "https://zlbb.faendir.com";
    let enc = urlencoding::encode(&since);

    // Phase 1: new records
    {
        let url = format!("{}/{}/records/new/{}", base, ctrl, enc);
        println!("[SYNC_NEW]: GET {}", url);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw_count = 0usize; let mut parsed: Vec<OmRecordDTO> = Vec::new();
                if let Ok(body) = res.text().await {
                    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                        raw_count = values.len();
                        for v in values {
                            match serde_json::from_value::<OmRecordDTO>(v) {
                                Ok(r) => parsed.push(r),
                                Err(e) => eprintln!("[SYNC_NEW_SKIP]: {}", e),
                            }
                        }
                    } else { errors.push("new: not JSON array".into()); }
                } else { errors.push("new: read failed".into()); }
                let skipped = raw_count.saturating_sub(parsed.len());
                if !parsed.is_empty() {
                    let mut vault = state.record_vault.lock().unwrap();
                    for r in &parsed {
                        let exists = vault.iter().any(|v| v.id.as_deref() == r.id.as_deref()
                            && v.full_formatted_score.as_deref() == r.full_formatted_score.as_deref());
                        if !exists { vault.push(r.clone()); added += 1; }
                    }
                }
                println!("[SYNC_NEW]: {} raw, {} ok, {} skip, {} added", raw_count, parsed.len(), skipped, added);
            }
            Ok(res) => errors.push(format!("new HTTP {}", res.status())),
            Err(e) => errors.push(format!("new fetch: {}", e)),
        }
    }

    // Phase 2: changes
    {
        let url = format!("{}/{}/records/changes/{}", base, ctrl, enc);
        println!("[SYNC_CHANGES]: GET {}", url);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw_count = 0usize; let mut parsed: Vec<OmRecordChange> = Vec::new();
                if let Ok(body) = res.text().await {
                    if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                        raw_count = values.len();
                        for v in values {
                            match serde_json::from_value::<OmRecordChange>(v) {
                                Ok(ch) => parsed.push(ch),
                                Err(e) => eprintln!("[SYNC_CHANGE_SKIP]: {}", e),
                            }
                        }
                    } else { errors.push("changes: not JSON array".into()); }
                } else { errors.push("changes: read failed".into()); }
                let mut vault = state.record_vault.lock().unwrap();
                for ch in &parsed {
                    match ch.r#type.as_str() {
                        "ADD" => {
                            let exists = vault.iter().any(|v| v.id.as_deref() == ch.record.id.as_deref()
                                && v.full_formatted_score.as_deref() == ch.record.full_formatted_score.as_deref());
                            if !exists { vault.push(ch.record.clone()); added += 1; }
                        }
                        "REMOVE" => {
                            if let Some(ref rid) = ch.record.id {
                                let before = vault.len();
                                vault.retain(|v| v.id.as_deref() != Some(rid.as_str()));
                                removed += before - vault.len();
                            }
                        }
                        _ => {}
                    }
                }
                println!("[SYNC_CHANGES]: {} raw, {} ok", raw_count, parsed.len());
            }
            Ok(res) => errors.push(format!("changes HTTP {}", res.status())),
            Err(e) => errors.push(format!("changes fetch: {}", e)),
        }
    }

    let synced_until = utc_now_iso();
    save_cache_meta(&app);
    println!("[SYNC_DONE]: +{} -{} errs={} until={}", added, removed, errors.len(), synced_until);
    Ok(SyncResult { new_count: added, removed_count: removed, synced_until, errors })
}

// ================= Pareto 实时判定引擎 =================

fn is_dominated_by(
    a_cost: Option<i32>, a_cycles: Option<i32>, a_area: Option<i32>, a_instructions: Option<i32>,
    b_cost: i32, b_cycles: i32, b_area: i32, b_instructions: i32, dims: &[&str],
) -> bool {
    let mut any_worse = false;
    for dim in dims {
        match *dim {
            "cost" => { let a = a_cost.unwrap_or(i32::MAX); if a < b_cost { return false; } if a > b_cost { any_worse = true; } }
            "cycles" => { let a = a_cycles.unwrap_or(i32::MAX); if a < b_cycles { return false; } if a > b_cycles { any_worse = true; } }
            "area" => { let a = a_area.unwrap_or(i32::MAX); if a < b_area { return false; } if a > b_area { any_worse = true; } }
            "instructions" => { let a = a_instructions.unwrap_or(i32::MAX); if a < b_instructions { return false; } if a > b_instructions { any_worse = true; } }
            _ => {}
        }
    }
    any_worse
}

fn is_exact_match(
    a_cost: Option<i32>, a_cycles: Option<i32>, a_area: Option<i32>, a_instructions: Option<i32>,
    b_cost: i32, b_cycles: i32, b_area: i32, b_instructions: i32, dims: &[&str],
) -> bool {
    for dim in dims {
        match *dim {
            "cost" => { if a_cost != Some(b_cost) { return false; } }
            "cycles" => { if a_cycles != Some(b_cycles) { return false; } }
            "area" => { if a_area != Some(b_area) { return false; } }
            "instructions" => { if a_instructions != Some(b_instructions) { return false; } }
            _ => {}
        }
    }
    true
}

fn make_diff(user_val: i32, best_val: i32) -> BeatenMetricDiff {
    let abs_diff = user_val - best_val;
    let pct = if best_val > 0 { (user_val as f64) / (best_val as f64) * 100.0 } else { f64::INFINITY };
    BeatenMetricDiff { actual_value: best_val, absolute_diff: abs_diff, percentage_diff: pct,
        formatted_string: format!("{} ({:+} / {:.2}%)", best_val, -abs_diff, pct) }
}

#[tauri::command]
async fn judge_draft(
    draft: OmDraftInput, mode: InputMode, puzzle_id: String,
    state: tauri::State<'_, MemoryState>,
) -> Result<JudgeResult, String> {
    let dims: Vec<&str> = match mode {
        InputMode::GCA => vec!["cost", "cycles", "area"],
        InputMode::GCAI => vec!["cost", "cycles", "area", "instructions"],
    };
    if (dims.contains(&"cost") && draft.cost.is_none())
        || (dims.contains(&"cycles") && draft.cycles.is_none())
        || (dims.contains(&"area") && draft.area.is_none())
        || (dims.contains(&"instructions") && draft.instructions.is_none())
    { return Ok(JudgeResult { status: ParetoJudgeStatus::Unknown, total_compared: 0, report: None }); }

    let vault = state.record_vault.lock().unwrap();
    let candidates: Vec<OmRecordDTO> = vault.iter().filter(|r| {
        r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)
            && r.score.is_some()
            && r.score.as_ref().unwrap().overlap == draft.overlap
            && r.score.as_ref().unwrap().trackless == draft.trackless
    }).cloned().collect();
    let total = candidates.len();
    println!("[JUDGE]: p={} mode={:?} o={} t={} cand={}", puzzle_id, mode, draft.overlap, draft.trackless, total);
    if total == 0 { return Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: 0, report: None }); }

    for r in &candidates {
        let s = r.score.as_ref().unwrap();
        if is_exact_match(draft.cost, draft.cycles, draft.area, draft.instructions,
            s.cost, s.cycles, s.area, s.instructions, &dims)
        { return Ok(JudgeResult { status: ParetoJudgeStatus::AlreadyPresented, total_compared: total, report: None }); }
    }

    let dominators: Vec<&OmRecordDTO> = candidates.iter().filter(|r| {
        let s = r.score.as_ref().unwrap();
        is_dominated_by(draft.cost, draft.cycles, draft.area, draft.instructions,
            s.cost, s.cycles, s.area, s.instructions, &dims)
    }).collect();
    if dominators.is_empty() { return Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: total, report: None }); }

    let best = dominators.iter().fold(None, |acc: Option<(i32,i32,i32,i32)>, r| {
        let s = r.score.as_ref().unwrap();
        match acc { None => Some((s.cost, s.cycles, s.area, s.instructions)),
            Some((c,cy,a,i)) => Some((c.min(s.cost), cy.min(s.cycles), a.min(s.area), i.min(s.instructions))) }
    });
    let rep = dominators.iter().max_by(|a, b| {
        let sa = a.score.as_ref().unwrap(); let sb = b.score.as_ref().unwrap();
        (sb.cost+sb.cycles+sb.area+sb.instructions).cmp(&(sa.cost+sa.cycles+sa.area+sa.instructions))
    });
    let report = if let (Some((b_c,b_cy,b_a,b_i)), Some(rep_r)) = (best, rep) {
        println!("[JUDGE]: NothingBeaten by {} recs best=({},{},{},{})", dominators.len(), b_c,b_cy,b_a,b_i);
        Some(ParetoBeatenReport { better_record: rep_r.clone(),
            cost_diff: if dims.contains(&"cost") { Some(make_diff(draft.cost.unwrap_or(0), b_c)) } else { None },
            cycles_diff: if dims.contains(&"cycles") { Some(make_diff(draft.cycles.unwrap_or(0), b_cy)) } else { None },
            area_diff: if dims.contains(&"area") { Some(make_diff(draft.area.unwrap_or(0), b_a)) } else { None },
            instructions_diff: if dims.contains(&"instructions") { Some(make_diff(draft.instructions.unwrap_or(0), b_i)) } else { None },
        })
    } else { None };
    Ok(JudgeResult { status: ParetoJudgeStatus::NothingBeaten, total_compared: total, report })
}
`;

// 在 save_cache_meta 函数之后插入
const afterSaveMeta = 'fn save_cache_meta(app: &tauri::AppHandle) {\n    if let Ok(dir) = app.path().app_cache_dir() {\n        let now = utc_now_iso();\n        let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));\n    }\n}';
if (!c.includes('fn sync_incremental')) {
    c = c.replace(afterSaveMeta, afterSaveMeta + '\n' + syncAndJudge);
    console.log('✓ sync+judge inserted');
}

// ═══ 10. 注册新命令 ═══
c = c.replace(
    'sync_incremental, get_live_puzzle_suggestions,',
    'sync_incremental, judge_draft, get_live_puzzle_suggestions,'
);

fs.writeFileSync('src/lib.rs', c);
console.log('DONE — all changes applied, length=' + c.length);
